import { useEffect, useRef } from "react";
import { useNavigate } from "react-router-dom";
import { GlassButton, Modal } from "../../design-system";
import { ComputePanel, useComputeStatus } from "../../features/compute";
import { usePrivateAiState } from "../../features/private-ai/public";
import { ProfilePage } from "../../features/profile";
import type { usePublicProfile } from "../../features/profile/public";
import { SettingsPage, settingsSections } from "../../features/settings";
import type { useCoreStatus } from "../../lib/tauri/use-core-status";
import { flowPaths, routePaths } from "../routes";

function scrollToSection(body: HTMLElement | null, id: string) {
  const target = body?.querySelector<HTMLElement>(`[data-sec="${id}"]`);
  if (body && target) body.scrollTop = target.offsetTop - 6;
}

/**
 * Settings as the design's modal: section navigation on the left, the
 * sections in one scrolling body on the right. The public profile and the
 * Private AI pointer are sections of it.
 */
export function SettingsModal({
  open,
  section,
  onClose,
  core,
  publicProfile,
}: {
  open: boolean;
  section: string | null;
  onClose: () => void;
  core: ReturnType<typeof useCoreStatus>;
  publicProfile: ReturnType<typeof usePublicProfile>;
}) {
  const navigate = useNavigate();
  const bodyRef = useRef<HTMLDivElement>(null);
  const jump = (id: string) => scrollToSection(bodyRef.current, id);
  useEffect(() => {
    if (!open || !section) return;
    const frame = requestAnimationFrame(() =>
      scrollToSection(bodyRef.current, section),
    );
    return () => cancelAnimationFrame(frame);
  }, [open, section]);

  return (
    <Modal
      open={open}
      onClose={onClose}
      title="Settings"
      subtitle="What this machine watches, and what your traces are allowed to do."
      headerAccessory={
        core.data ? (
          <span className="tc-chip" style={{ boxShadow: "none" }}>
            <svg
              width="11"
              height="11"
              viewBox="0 0 16 16"
              fill="none"
              stroke="currentColor"
              strokeWidth="1.4"
              aria-hidden="true"
            >
              <path d="M1.5 8C3 5.2 5.2 3.7 8 3.7s5 1.5 6.5 4.3C13 10.8 10.8 12.3 8 12.3S3 10.8 1.5 8z" />
              <circle cx="8" cy="8" r="2.1" />
            </svg>
            {core.data.daemon.paused ? "Paused" : "Watching"}
          </span>
        ) : null
      }
      className="h-full"
      bodyClassName="grid grid-cols-[180px_minmax(0,1fr)] min-h-0"
    >
      <nav
        aria-label="Settings sections"
        className="flex flex-col gap-px overflow-auto px-2 py-2.5"
        style={{ borderRight: "0.5px solid rgba(255,255,255,0.1)" }}
      >
        {settingsSections.map((item) => (
          <button
            key={item.id}
            type="button"
            className="truncate rounded-lg border-0 bg-transparent px-2.5 py-1.5 text-left text-[12px] tc-text-secondary hover:bg-white/8 hover:text-[var(--tc-text-primary)]"
            onClick={() => jump(item.id)}
          >
            {item.label}
          </button>
        ))}
      </nav>
      <div ref={bodyRef} className="relative min-h-0 overflow-auto px-5 pt-3.5 pb-6">
        <SettingsPage
          key={core.scope}
          onTurnOnAutomaticContributing={() => {
            onClose();
            navigate(flowPaths["automatic-contributing"]);
          }}
          profile={
            <ProfilePage
              coreStatus={core.data}
              coreStatusState={core.state}
              onRefresh={core.refresh}
              publicProfile={publicProfile.data}
              publicProfileState={publicProfile.state}
              onPublicProfileRefresh={publicProfile.refresh}
            />
          }
          privateAi={
            <div className="flex flex-wrap items-center justify-between gap-3">
              <PrivateAiStateLine />
              <GlassButton
                onClick={() => {
                  onClose();
                  navigate(routePaths["private-ai"]);
                }}
              >
                Private AI
              </GlassButton>
            </div>
          }
          compute={<ComputeSection />}
        />
      </div>
    </Modal>
  );
}

/** The core's sentence for what Private AI is doing now. */
function PrivateAiStateLine() {
  const state = usePrivateAiState();
  return (
    <span className="tc-label min-w-0 flex-1 font-normal">
      {state.line ?? "—"}
    </span>
  );
}

/**
 * Compute consent, reachable whatever else is on screen: a consent granted
 * in any build can be paused, resumed and withdrawn here.
 */
function ComputeSection() {
  const compute = useComputeStatus();
  if (compute.state === "loading") {
    return (
      <p className="m-0 tc-body tc-text-tertiary" role="status">
        Reading compute settings…
      </p>
    );
  }
  return <ComputePanel compute={compute} />;
}
