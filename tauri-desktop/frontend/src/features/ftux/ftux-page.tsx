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
import { GlassWindow } from "./components/glass";
import type { AsyncState } from "./components/join-screen";
import { JoinScreen } from "./components/join-screen";
import { PasskeyFlow, WelcomeBack } from "./components/passkey-flow";
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

function initialSelections(
  repos: RepoCandidate[],
  defaults: Record<string, number[]>,
): RepoSelection[] {
  return repos.map((repo) => {
    const picked = new Set(defaults[repo.folder] ?? []);
    return {
      folder: repo.folder,
      rule: repo.defaultRule,
      sessionCount: repo.sessions.length,
      selected: repo.sessions.map(
        (_, index) => repo.defaultRule !== "never" && picked.has(index),
      ),
    };
  });
}

export function FtuxPage({
  initialPath = "connect",
  initialScreen = "join",
  initialInvite = null,
  returningPasskey = null,
  onComplete,
}: {
  initialPath?: FtuxPath;
  initialScreen?: FtuxScreen;
  initialInvite?: string | null;
  // Set when a passkey is already stored on this Mac (P-7).
  returningPasskey?: string | null;
  onComplete: (settings: FtuxSettings) => void;
}) {
  const [path, setPath] = useState<FtuxPath>(initialPath);
  const [screen, setScreen] = useState<FtuxScreen>(initialScreen);

  const [join, setJoin] = useState<JoinState>({
    invite: null,
    passkey: null,
    nearAi: false,
  });
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

  const [optionalUses, setOptionalUses] = useState<boolean[]>(
    OPTIONAL_USES.map(() => true),
  );
  const [listHandle, setListHandle] = useState(false);
  const [sharing, setSharing] = useState<SharingMode>("auto");
  const [privateAi, setPrivateAi] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    void detectTools().then((found) => {
      if (live) setTools(found);
    });
    void listRepoCandidates().then(({ repos: found, defaultSelection }) => {
      if (!live) return;
      setCandidates(found);
      setRepos(initialSelections(found, defaultSelection));
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

  const handleAddTool = async (droppedName?: string) => {
    const folder = droppedName ?? (await chooseFolder());
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
    setWatch((current) => ({ ...current, [id]: "watch" }));
  };

  const updateRepo = (
    folder: string,
    update: (repo: RepoSelection) => RepoSelection,
  ) =>
    setRepos((current) =>
      current.map((repo) => (repo.folder === folder ? update(repo) : repo)),
    );

  const handleStart = async () => {
    const settings: FtuxSettings = {
      path,
      join,
      watch,
      customFolders,
      repos,
      optionalUses,
      listHandle,
      sharing,
      privateAi: path === "customize" && privateAi,
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

  const watchedSource =
    tools?.find((tool) => watch[tool.id] === "watch")?.name ?? "Claude Code";

  return (
    <div className="ftux">
      <span className="ftux-preview-tag" title="Backend calls are mocked">
        PREVIEW · MOCK DATA
      </span>
      <GlassWindow
        eyebrow={
          path === "connect" ? "Getting started" : "Getting started · customize"
        }
        steps={steps}
        current={stepIndex(path, screen)}
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
          />
        ) : null}
        {screen === "folders" ? (
          <FoldersScreen
            {...toolProps}
            onCustomize={() => changePath("customize")}
          />
        ) : null}
        {screen === "tools" ? (
          <ToolsScreen
            {...toolProps}
            onAddTool={(name) => void handleAddTool(name)}
          />
        ) : null}
        {screen === "rules" ? (
          <RulesScreen
            candidates={candidates}
            selections={repos}
            sourceName={watchedSource}
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
            optionalUses={optionalUses}
            listHandle={listHandle}
            sharing={sharing}
            privateAi={path === "customize" ? privateAi : null}
            submitting={submitting}
            error={submitError}
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
      </GlassWindow>

      {welcomeBack && returningPasskey ? (
        <WelcomeBack
          passkeyName={returningPasskey}
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
          onDone={(passkey) => {
            setPasskeyOpen(null);
            setJoin((current) => ({ ...current, passkey }));
          }}
          onClose={(reason) => {
            setPasskeyOpen(null);
            if (reason === "signed-out") {
              setNotice(
                "Passkey not verified, so you were signed out. Create one again whenever you like.",
              );
            }
          }}
        />
      ) : null}
    </div>
  );
}
