#!/usr/bin/env python3
"""Validate, preview, and explicitly publish one environment's feature manifest."""
import argparse
import hashlib
import json
import pathlib
import re
import subprocess
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
        return generation,json.loads(raw,object_pairs_hook=unique)
    def write(self,target,path,generation):
        subprocess.run(self.command("cp","--content-type=application/json","--cache-control=public,max-age=60","--if-generation-match="+generation,str(path),target),check=True,timeout=60)

def rust_validator(binary):
    def validate(path,environment):
        output=subprocess.check_output([str(binary),"feature-flags","validate","--file",str(path),"--environment",environment],timeout=15)
        return json.loads(output,object_pairs_hook=unique)
    return validate

def execute(environment,path,storage,validate,apply=False,commit="unknown"):
    target=TARGETS[environment]
    with pathlib.Path(path).open("rb") as stream:raw=stream.read(MAX_BYTES+1)
    if len(raw)>MAX_BYTES:raise ValueError("manifest-too-large")
    # Snapshot the reviewed bytes; validation and upload never re-open the mutable input.
    with tempfile.TemporaryDirectory(prefix="feature-manifest-") as directory:
        immutable=pathlib.Path(directory)/"manifest.json"
        immutable.write_bytes(raw)
        candidate=validate(immutable,environment)
        if candidate.get("environment")!=environment:raise ValueError("manifest-environment-mismatch")
        generation,previous=storage.read(target)
        if previous is not None:
            if previous.get("environment")!=environment or previous.get("schema_version")!=1 or type(previous.get("revision"))is not int:
                raise ValueError("invalid-current-manifest")
            if candidate["revision"]<=previous["revision"]:raise ValueError("publish-requires-increasing-revision")
        before=previous.get("flags",{}) if previous else {}
        after=candidate["flags"]
        report=dict(environment=environment,target=target,revision=candidate["revision"],expected_generation=generation,sha256=hashlib.sha256(raw).hexdigest(),commit=commit,published=False,changes={k:{"before":before.get(k),"after":after.get(k)} for k in sorted(set(before)|set(after)) if before.get(k)!=after.get(k)})
        print(json.dumps(report,indent=2),flush=True)
        if apply:
            # Re-check freshness directly before the conditional replacement.
            validate(immutable,environment)
            storage.write(target,immutable,generation)
            report["published"]=True
        return report

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--environment",choices=TARGETS,required=True)
    parser.add_argument("--manifest",type=pathlib.Path,required=True)
    parser.add_argument("--cli",type=pathlib.Path,required=True)
    parser.add_argument("--account",help="Already authenticated gcloud account; does not change defaults")
    parser.add_argument("--publish",action="store_true")
    args=parser.parse_args()
    try:
        commit=subprocess.check_output(["git","-C",str(ROOT),"rev-parse","HEAD"],text=True).strip()
        result=execute(args.environment,args.manifest,GcloudStorage(args.account),rust_validator(args.cli.resolve()),args.publish,commit)
        if result["published"]:print("Published revision",result["revision"],"to",result["environment"])
    except (ValueError,KeyError,OSError,subprocess.SubprocessError) as error:parser.error(str(error))

if __name__=="__main__":main()
