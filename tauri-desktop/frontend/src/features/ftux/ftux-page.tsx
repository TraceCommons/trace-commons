import "./ftux.css";
import { useEffect, useState } from "react";
import {
  chooseFolder,
  detectTools,
  finishSetup,
  listRepoCandidates,
  lookupInvite,
  openExternal,
  signInWithNearAi,
} from "./api/ftux-api";
import { FtuxFrame } from "./components/ftux-frame";
import type { AsyncState } from "./components/join-screen";
import { JoinScreen } from "./components/join-screen";
import {
  PasskeyFlow,
  type PasskeyResult,
  WelcomeBack,
} from "./components/passkey-flow";
import { RulesScreen } from "./components/rules-screen";
import { FoldersScreen, ToolsScreen } from "./components/tool-screens";
import { UsesScreen } from "./components/uses-screen";
import {
  applyRule,
  canContinueFromFolders,
  type FtuxPath,
  type FtuxScreen,
  nextScreen,
  OPTIONAL_USES,
  type RepoSelection,
  reposForWatchedTools,
  stepIndex,
  stepsFor,
  switchPath,
  toggleAt,
  toggleGroup,
  type WatchAnswer,
} from "./ftux-model";
import type {
  DetectedTool,
  FtuxSettings,
  JoinState,
  RepoCandidate,
  SharingMode,
} from "./types";

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : "Something went wrong.";
}

// No past session starts ticked: including one is always the person's choice.
function initialSelections(repos: RepoCandidate[]): RepoSelection[] {
  return repos.map((repo) => ({
    folder: repo.folder,
    rule: repo.defaultRule,
    sessionCount: repo.sessions.length,
    selected: repo.sessions.map(() => false),
  }));
}

// Rules are only set on Custom setup, and only for repos whose tool
// is watched; nothing the person has not seen goes into setup.
function offeredRepos(
  candidates: RepoCandidate[] | null,
  repos: RepoSelection[],
  watch: Record<string, WatchAnswer>,
  tools: DetectedTool[] | null,
) {
  const offered =
    candidates === null ? null : reposForWatchedTools(candidates, watch);
  const folders = new Set((offered ?? []).map((repo) => repo.folder));
  const names = new Set(
    (offered ?? []).map(
      (repo) =>
        tools?.find((tool) => tool.id === repo.sourceToolId)?.name ??
        repo.sourceToolId,
    ),
  );
  return {
    candidates: offered,
    repos: repos.filter((repo) => folders.has(repo.folder)),
    sourceName: [...names].join(" and "),
  };
}

const SIGNED_OUT: JoinState = { invite: null, passkey: null, nearAi: false };

export function FtuxPage({
  initialPath = "quick",
  initialScreen = "join",
  initialInvite = null,
  returningPasskey = null,
  showPasskey = false,
  onComplete,
}: {
  initialPath?: FtuxPath;
  initialScreen?: FtuxScreen;
  initialInvite?: string | null;
  // Set when a passkey is already stored on this Mac (P-7).
  returningPasskey?: PasskeyResult | null;
  // The passkey card on Join; hidden in the first release (see JoinScreen).
  showPasskey?: boolean;
  onComplete: (settings: FtuxSettings) => void;
}) {
  const [path, setPath] = useState<FtuxPath>(initialPath);
  const [screen, setScreen] = useState<FtuxScreen>(initialScreen);

  const [join, setJoin] = useState<JoinState>(SIGNED_OUT);
  const [inviteDraft, setInviteDraft] = useState(initialInvite ?? "");
  const [lookup, setLookup] = useState<AsyncState>({ status: "idle" });
  const [nearAi, setNearAi] = useState<AsyncState>({ status: "idle" });
  const [notice, setNotice] = useState<string | null>(null);
  const [passkeyOpen, setPasskeyOpen] = useState<"choose" | "sign-in" | null>(
    null,
  );
  const [welcomeBack, setWelcomeBack] = useState(returningPasskey !== null);

  const [tools, setTools] = useState<DetectedTool[] | null>(null);
  const [watch, setWatch] = useState<Record<string, WatchAnswer>>({});
  const [customFolders, setCustomFolders] = useState<Record<string, string>>(
    {},
  );
  const [candidates, setCandidates] = useState<RepoCandidate[] | null>(null);
  const [repos, setRepos] = useState<RepoSelection[]>([]);

  const [baseUse, setBaseUse] = useState(false);
  const [optionalUses, setOptionalUses] = useState<boolean[]>(
    OPTIONAL_USES.map(() => false),
  );
  const [listHandle, setListHandle] = useState(false);
  const [sharing, setSharing] = useState<SharingMode>("ask");
  const [privateAi, setPrivateAi] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void detectTools().then((found) => {
      if (live) setTools(found);
    });
    void listRepoCandidates().then((found) => {
      if (!live) return;
      setCandidates(found);
      setRepos(initialSelections(found));
    });
    return () => {
      live = false;
    };
  }, []);

  const steps = stepsFor(path);
  const advance = () => {
    const next = nextScreen(path, screen);
    if (next) setScreen(next);
  };
  const changePath = (to: FtuxPath) => {
    const moved = switchPath(to, screen);
    setPath(moved.path);
    setScreen(moved.screen);
  };

  const handleLookup = async () => {
    setLookup({ status: "busy" });
    try {
      const invite = await lookupInvite(inviteDraft);
      setJoin((current) => ({ ...current, invite }));
      setLookup({ status: "idle" });
    } catch (error) {
      setLookup({ status: "error", error: errorMessage(error) });
    }
  };

  const handleNearAi = async () => {
    setNearAi({ status: "busy" });
    try {
      await signInWithNearAi();
      setJoin((current) => ({ ...current, nearAi: true }));
      setNearAi({ status: "idle" });
    } catch (error) {
      setNearAi({ status: "error", error: errorMessage(error) });
    }
  };

  const handleChooseFolder = async (toolId: string) => {
    const folder = await chooseFolder();
    if (!folder) return;
    setCustomFolders((current) => ({ ...current, [toolId]: folder }));
    setWatch((current) => ({ ...current, [toolId]: "watch" }));
  };

  // A drop carries no path the page can read, so it opens the picker too.
  // An added tool starts unanswered, like every other row.
  const handleAddTool = async () => {
    const folder = await chooseFolder();
    if (!folder) return;
    const id = `custom:${folder}`;
    setTools((current) =>
      current?.some((tool) => tool.id === id)
        ? current
        : [
            ...(current ?? []),
            {
              id,
              badge: "+",
              name: folder.split("/").filter(Boolean).pop() ?? folder,
              folder,
              presence: "found",
              sessionCount: 0,
              detail: "Added by you",
              custom: true,
            },
          ],
    );
  };

  const updateRepo = (
    folder: string,
    update: (repo: RepoSelection) => RepoSelection,
  ) =>
    setRepos((current) =>
      current.map((repo) => (repo.folder === folder ? update(repo) : repo)),
    );

  const offer = offeredRepos(candidates, repos, watch, tools);

  const handleStart = async () => {
    const settings: FtuxSettings = {
      path,
      join,
      watch,
      customFolders,
      baseUse,
      repos: path === "custom" ? offer.repos : [],
      optionalUses,
      listHandle,
      sharing,
      privateAi: path === "custom" && privateAi,
    };
    setSubmitting(true);
    setSubmitError(null);
    try {
      await finishSetup(settings);
      onComplete(settings);
    } catch (error) {
      setSubmitError(errorMessage(error));
    } finally {
      setSubmitting(false);
    }
  };

  const toolProps = {
    tools,
    answers: watch,
    customFolders,
    canContinue: tools !== null && canContinueFromFolders(tools, watch),
    onAnswer: (toolId: string, answer: WatchAnswer) =>
      setWatch((current) => ({ ...current, [toolId]: answer })),
    onChooseFolder: (toolId: string) => void handleChooseFolder(toolId),
    onInstall: (url: string) => void openExternal(url),
    onContinue: advance,
  };

  const overlays = (
    <>
      {welcomeBack && returningPasskey ? (
        <WelcomeBack
          passkeyName={returningPasskey.name}
          onSignIn={() => {
            setWelcomeBack(false);
            setPasskeyOpen("sign-in");
          }}
          onOtherOptions={() => setWelcomeBack(false)}
        />
      ) : null}

      {passkeyOpen ? (
        <PasskeyFlow
          initialStep={passkeyOpen}
          storedPasskey={returningPasskey}
          onDone={(passkey) => {
            setPasskeyOpen(null);
            setJoin((current) => ({ ...current, passkey }));
          }}
          onClose={(reason) => {
            setPasskeyOpen(null);
            if (reason === "signed-out") {
              // Signing out undoes every sign-in on this screen, not only the
              // passkey, so no card claims an account that is not linked.
              setJoin(SIGNED_OUT);
              setInviteDraft("");
              setNotice(
                "Passkey not verified, so you were signed out. Join again whenever you like.",
              );
            }
          }}
        />
      ) : null}
    </>
  );

  return (
    <FtuxFrame
      eyebrow={path === "quick" ? "Quick setup" : "Custom setup"}
      steps={steps}
      current={stepIndex(path, screen)}
      overlays={overlays}
    >
      {screen === "join" ? (
        <JoinScreen
          join={join}
          inviteDraft={inviteDraft}
          lookup={lookup}
          nearAi={nearAi}
          notice={notice}
          onInviteDraft={(value) => {
            setInviteDraft(value);
            if (lookup.status === "error") setLookup({ status: "idle" });
          }}
          onLookup={() => void handleLookup()}
          onCreatePasskey={() => {
            setNotice(null);
            setPasskeyOpen("choose");
          }}
          onSignInNearAi={() => void handleNearAi()}
          onNext={advance}
          showPasskey={showPasskey}
        />
      ) : null}
      {screen === "folders" ? (
        <FoldersScreen {...toolProps} onCustom={() => changePath("custom")} />
      ) : null}
      {screen === "tools" ? (
        <ToolsScreen {...toolProps} onAddTool={() => void handleAddTool()} />
      ) : null}
      {screen === "rules" ? (
        <RulesScreen
          candidates={offer.candidates}
          selections={offer.repos}
          sourceName={offer.sourceName}
          onRule={(folder, rule) =>
            updateRepo(folder, (repo) => applyRule(repo, rule))
          }
          onToggleAll={(folder) =>
            updateRepo(folder, (repo) => ({
              ...repo,
              selected: toggleGroup(repo.selected),
            }))
          }
          onToggleSession={(folder, index) =>
            updateRepo(folder, (repo) => ({
              ...repo,
              selected: toggleAt(repo.selected, index),
            }))
          }
          onContinue={advance}
        />
      ) : null}
      {screen === "uses" ? (
        <UsesScreen
          baseUse={baseUse}
          optionalUses={optionalUses}
          listHandle={listHandle}
          sharing={sharing}
          privateAi={path === "custom" ? privateAi : null}
          submitting={submitting}
          error={submitError}
          onToggleBaseUse={() => setBaseUse(!baseUse)}
          onToggleUses={() => setOptionalUses(toggleGroup(optionalUses))}
          onToggleUse={(index) =>
            setOptionalUses(toggleAt(optionalUses, index))
          }
          onToggleHandle={() => setListHandle(!listHandle)}
          onSharing={setSharing}
          onTogglePrivateAi={() => setPrivateAi(!privateAi)}
          onStart={() => void handleStart()}
        />
      ) : null}
    </FtuxFrame>
  );
}
