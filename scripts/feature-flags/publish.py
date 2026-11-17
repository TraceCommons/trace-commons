#!/usr/bin/env python3
"""Validate, preview, and explicitly publish one environment's feature manifest."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
import sys
import tempfile

ROOT=pathlib.Path(__file__).resolve().parents[2]
TARGETS={
    "production":"gs://tracecommons-flatpak/feature-flags/production.json",
    "staging":"gs://tc-staging-updates-20261022/feature-flags/staging.json",
    "testing":"gs://tc-staging-updates-20261022/feature-flags/testing.json",
}
MAX_BYTES=65536

def unique(pairs):
    result={}
    for key,value in pairs:
        if key in result:raise ValueError("duplicate-manifest-key")
        result[key]=value
    return result

class GcloudStorage:
    def __init__(self,account=None):self.account=account
    def command(self,*args):
        command=["gcloud","storage",*args,"--verbosity=error"]
        if self.account:command.append("--account="+self.account)
        return command
    def read(self,target):
        result=subprocess.run(self.command("objects","describe",target,"--format=json"),capture_output=True,text=True,timeout=30)
        if result.returncode:
            if re.search(r"\b404\b",result.stderr):return "0",None
            raise ValueError("could-not-read-current-manifest-metadata")
        metadata=json.loads(result.stdout)
        generation=str(metadata["generation"])
        if not generation.isdigit():raise ValueError("invalid-object-generation")
        if int(metadata.get("size",0))>MAX_BYTES:raise ValueError("current-manifest-too-large")
        raw=subprocess.check_output(self.command("cat",target+"#"+generation),timeout=30)
        if len(raw)>MAX_BYTES:raise ValueError("current-manifest-too-large")
        return generation,raw
    def write(self,target,path,generation):
        subprocess.run(self.command("cp","--content-type=application/json","--cache-control=public,max-age=60","--if-generation-match="+generation,str(path),target),check=True,timeout=60)

def rust_validator(binary):
    def validate(path,environment):
        output=subprocess.check_output([str(binary),"feature-flags","validate","--file",str(path),"--environment",environment],timeout=15)
        return json.loads(output,object_pairs_hook=unique)
    return validate

def current_manifest(raw):
    """The manifest inside the current object, signed or from before signing.

    Read only for its revision and flags: the publisher's revision floor and
    change report. Its signature is not what this run relies on."""
    document=json.loads(raw,object_pairs_hook=unique)
    if isinstance(document,dict) and "payload" in document:
        if not isinstance(document["payload"],str):raise ValueError("invalid-current-manifest")
        document=json.loads(document["payload"],object_pairs_hook=unique)
    if not isinstance(document,dict):raise ValueError("invalid-current-manifest")
    return document

class KmsSigner:
    """Sign through Cloud KMS. There is no key file: the key never leaves KMS."""
    def __init__(self,cli,location,keyring,key,version,public_key_pem,project=None,account=None):
        self.cli,self.location,self.keyring,self.key,self.version=cli,location,keyring,key,version
        self.public_key_pem,self.project,self.account=public_key_pem,project,account
    def __call__(self,manifest,environment):
        with tempfile.TemporaryDirectory(prefix="feature-sign-") as directory:
            directory=pathlib.Path(directory)
            signing_input,signature,document=directory/"input",directory/"signature",directory/"signed.json"
            subprocess.run([str(self.cli),"feature-flags","signing-input","--file",str(manifest),"--environment",environment,"--output",str(signing_input)],check=True,timeout=15)
            command=["gcloud","kms","asymmetric-sign","--location="+self.location,"--keyring="+self.keyring,"--key="+self.key,"--version="+str(self.version),"--input-file="+str(signing_input),"--signature-file="+str(signature),"--verbosity=error"]
            if self.project:command.append("--project="+self.project)
            if self.account:command.append("--account="+self.account)
            subprocess.run(command,check=True,timeout=60)
            subprocess.run([str(self.cli),"feature-flags","assemble","--file",str(manifest),"--environment",environment,"--signature",str(signature),"--public-key-pem",str(self.public_key_pem),"--output",str(document)],check=True,timeout=15)
            # Exactly what a client of this build checks.
            subprocess.run([str(self.cli),"feature-flags","verify","--file",str(document),"--environment",environment],check=True,timeout=15,capture_output=True)
            return document.read_bytes()

def run_git(*args):
    result=subprocess.run(["git","-C",str(ROOT),*args],capture_output=True,text=True,timeout=30)
    return result.returncode,result.stdout

def cli_version(binary):
    return subprocess.check_output([str(binary),"--version"],text=True,timeout=15)

def check_source(manifest,cli,git=run_git,version=cli_version):
    """Return (HEAD commit, problems) tying the publication to reviewed Git state."""
    code,out=git("rev-parse","HEAD")
    head=out.strip()
    if code or not re.fullmatch(r"[0-9a-f]{40,64}",head):raise ValueError("could-not-read-head-commit")
    problems=[]
    path=str(pathlib.Path(manifest).resolve())
    if git("ls-files","--error-unmatch","--",path)[0]:problems.append("manifest is not tracked by Git")
    elif git("status","--porcelain","--",path)[1].strip():problems.append("manifest has uncommitted changes")
    if git("merge-base","--is-ancestor","HEAD","origin/main")[0]:problems.append("HEAD is not contained in origin/main (fetch origin and publish from merged main)")
    try:
        # Format from trace-commons-build-info: "<ver> (commit <short>, built <time>)" plus an optional channel suffix.
        found=re.search(r"\(commit ([0-9A-Za-z._-]+),",version(cli))
        built=found.group(1) if found else None
    except (OSError,subprocess.SubprocessError):built=None
    if built is None:problems.append("cli build commit could not be read from --version")
    elif not re.fullmatch(r"[0-9a-f]{7,64}",built) or not head.startswith(built):problems.append("cli was not built from HEAD (cli commit "+built+")")
    return head,problems

def execute(environment,path,storage,validate,apply=False,commit="unknown",initial=False,signer=None):
    target=TARGETS[environment]
    with pathlib.Path(path).open("rb") as stream:raw=stream.read(MAX_BYTES+1)
    if len(raw)>MAX_BYTES:raise ValueError("manifest-too-large")
    # Snapshot the reviewed bytes; validation and upload never re-open the mutable input.
    with tempfile.TemporaryDirectory(prefix="feature-manifest-") as directory:
        immutable=pathlib.Path(directory)/"manifest.json"
        immutable.write_bytes(raw)
        candidate=validate(immutable,environment)
        if candidate.get("environment")!=environment:raise ValueError("manifest-environment-mismatch")
        generation,current=storage.read(target)
        previous=None if current is None else current_manifest(current)
        if previous is None and not initial:
            raise ValueError("current-manifest-missing: clients keep their highest accepted revision even after the object expires, and silently reject a lower revision (feature-manifest-rollback); republishing a low revision after a deletion would leave every client on defaults. Pass --initial only for the very first publish of this environment, with a revision above any ever published")
        if previous is not None and initial:
            raise ValueError("initial-refused: --initial is only for the first publish; a current manifest exists, so the revision must exceed its revision")
        if previous is not None:
            if previous.get("environment")!=environment or previous.get("schema_version")!=1 or type(previous.get("revision"))is not int:
                raise ValueError("invalid-current-manifest")
            if candidate["revision"]<=previous["revision"]:raise ValueError("publish-requires-increasing-revision")
        before=previous.get("flags",{}) if previous else {}
        after=candidate["flags"]
        report=dict(environment=environment,target=target,revision=candidate["revision"],expected_generation=generation,sha256=hashlib.sha256(raw).hexdigest(),commit=commit,signed=False,published=False,changes={k:{"before":before.get(k),"after":after.get(k)} for k in sorted(set(before)|set(after)) if before.get(k)!=after.get(k)})
        print(json.dumps(report,indent=2),flush=True)
        if apply:
            if signer is None:raise ValueError("signer-required: clients refuse an unsigned manifest")
            # Re-check freshness directly before signing and the conditional replacement.
            validate(immutable,environment)
            signed=pathlib.Path(directory)/"signed.json"
            signed.write_bytes(signer(immutable,environment))
            if len(signed.read_bytes())>MAX_BYTES:raise ValueError("signed-manifest-too-large")
            report["signed"]=True
            storage.write(target,signed,generation)
            report["published"]=True
        return report

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--environment",choices=TARGETS,required=True)
    parser.add_argument("--manifest",type=pathlib.Path,required=True)
    parser.add_argument("--cli",type=pathlib.Path,required=True)
    parser.add_argument("--account",help="Already authenticated gcloud account; does not change defaults")
    parser.add_argument("--publish",action="store_true")
    parser.add_argument("--initial",action="store_true",help="Allow publishing when no object exists; only for the very first publish of this environment")
    parser.add_argument("--kms-location")
    parser.add_argument("--kms-keyring")
    parser.add_argument("--kms-key")
    parser.add_argument("--kms-key-version",default="1")
    parser.add_argument("--kms-project")
    parser.add_argument("--public-key-pem",type=pathlib.Path,help="The signing key's PUBLIC KEY PEM; must be pinned in the environment's profile")
    args=parser.parse_args()
    try:
        commit,problems=check_source(args.manifest,args.cli.resolve())
        if problems and args.publish:raise ValueError("source-preconditions-failed: "+"; ".join(problems))
        for problem in problems:print("warning:",problem,file=sys.stderr)
        signer=None
        if args.publish:
            missing=[name for name in ("kms_location","kms_keyring","kms_key","public_key_pem") if getattr(args,name) is None]
            if missing:raise ValueError("signing-key-required: --"+", --".join(name.replace("_","-") for name in missing))
            signer=KmsSigner(args.cli.resolve(),args.kms_location,args.kms_keyring,args.kms_key,args.kms_key_version,args.public_key_pem,args.kms_project,args.account)
        result=execute(args.environment,args.manifest,GcloudStorage(args.account),rust_validator(args.cli.resolve()),args.publish,commit,args.initial,signer)
        if result["published"]:print("Published revision",result["revision"],"to",result["environment"])
    except (ValueError,KeyError,OSError,subprocess.SubprocessError) as error:parser.error(str(error))

if __name__=="__main__":main()
