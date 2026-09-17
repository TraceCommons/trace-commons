import { zodResolver } from "@hookform/resolvers/zod";
import { useEffect, useState } from "react";
import { useForm } from "react-hook-form";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { FormFieldError } from "../../../components/form-field-error";
import { ResponsiveOverlay } from "../../../components/responsive-overlay";
import {
  type OriginalSearchFormValues,
  originalSearchFormSchema,
} from "../forms";
import { usePreviewInspector } from "../hooks/use-preview-inspector";
import type { WaitingPreview } from "../types";
import { NativeReviewActions } from "./native-review-actions";
import { RedactedTranscript } from "./redacted-transcript";

type InspectorTab = "transcript" | "search" | "turns";

export function PreviewInspector({
  preview,
  open,
  onClose,
  onReviewed,
}: {
  preview: WaitingPreview | null;
  open: boolean;
  onClose: () => void;
  onReviewed?: () => void;
}) {
  const inspector = usePreviewInspector(
    open ? (preview?.entry.entry_id ?? null) : null,
  );
  const [tab, setTab] = useState<InspectorTab>("transcript");
  useEffect(() => {
    if (!open) setTab("transcript");
  }, [open]);
  if (!open || !preview) return null;
  const loaded = inspector.nextOffset === null && inspector.digest !== null;
  return (
    <ResponsiveOverlay
      open={open}
      onOpenChange={(nextOpen) => {
        if (!nextOpen) onClose();
      }}
      title="Exactly what would be sent"
      description="This is the redacted envelope. It stays local while you read it. Original-session search returns only a count; it never returns raw text."
      footer={
        <Button type="button" variant="outline" onClick={onClose}>
          Close
        </Button>
      }
    >
      <div className="grid gap-4">
        <span className="font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
          LOOK INSIDE
        </span>
        <p className="m-0 text-[11px] leading-[1.55] text-muted-foreground">
          This is the redacted envelope. It stays local while you read it.
          Original-session search returns only a count; it never returns raw
          text.
        </p>
        <div className="flex flex-wrap justify-between gap-x-[18px] gap-y-2 border border-border bg-muted px-3.5 py-3 text-[11px]">
          <strong>{preview.entry.source}</strong>
          <span>
            {formatBytes(preview.would_send_bytes)} would send ·{" "}
            {formatBytes(preview.raw_session_bytes)} on disk
          </span>
        </div>
        {onReviewed && (
          <NativeReviewActions
            entryId={preview.entry.entry_id}
            hasCertificate={preview.entry.holds_certificate === true}
            onReviewed={onReviewed}
          />
        )}
        {inspector.error && (
          <p className="rounded-[9px] border border-destructive/30 bg-destructive/10 px-3.5 py-3 text-[12px] text-destructive">
            {inspector.error}
          </p>
        )}
        {inspector.state === "idle" && (
          <Button type="button" onClick={() => void inspector.open()}>
            Load redacted transcript
          </Button>
        )}
        {inspector.state === "loading" && (
          <p className="text-[13px] text-muted-foreground">
            Loading bounded transcript…
          </p>
        )}
        {inspector.digest && (
          <>
            <div
              className="flex flex-wrap gap-1 border-b border-border"
              role="tablist"
              aria-label="Preview details"
            >
              <TabButton
                id="transcript"
                label="Exactly what would be sent"
                active={tab === "transcript"}
                onClick={setTab}
              />
              <TabButton
                id="search"
                label="Search original"
                active={tab === "search"}
                onClick={setTab}
              />
              <TabButton
                id="turns"
                label="Turn index"
                active={tab === "turns"}
                onClick={setTab}
              />
            </div>
            {tab === "transcript" && (
              <TranscriptTab inspector={inspector} loaded={loaded} />
            )}
            {tab === "search" && <SearchTab inspector={inspector} />}
            {tab === "turns" && (
              <TurnTab inspector={inspector} loaded={loaded} />
            )}
          </>
        )}
      </div>
    </ResponsiveOverlay>
  );
}

function TabButton({
  id,
  label,
  active,
  onClick,
}: {
  id: InspectorTab;
  label: string;
  active: boolean;
  onClick: (tab: InspectorTab) => void;
}) {
  return (
    <Button
      className={`border-0 border-b-2 border-transparent bg-transparent px-[11px] py-[9px] text-[11px] font-bold text-muted-foreground hover:border-primary hover:text-foreground${active ? " border-primary text-foreground" : ""}`}
      type="button"
      role="tab"
      aria-selected={active}
      onClick={() => onClick(id)}
    >
      {label}
    </Button>
  );
}

function TranscriptTab({
  inspector,
  loaded,
}: {
  inspector: ReturnType<typeof usePreviewInspector>;
  loaded: boolean;
}) {
  return (
    <div className="grid gap-3 border-0 pt-4" role="tabpanel">
      <p className="mb-3 text-[11px] leading-[1.5] text-muted-foreground">
        These are the exact redacted bytes an approval covers. Markers show
        where local scrubbing fired.
      </p>
      {inspector.body && (
        <RedactedTranscript body={inspector.body} turns={inspector.turns} />
      )}
      {inspector.nextOffset !== null && (
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="button"
          onClick={() => void inspector.loadMore()}
          disabled={inspector.state === "busy"}
        >
          Load more (
          {formatBytes(
            inspector.totalBytes -
              new TextEncoder().encode(inspector.body).byteLength,
          )}{" "}
          remaining)
        </Button>
      )}
      {loaded && inspector.turns.length === 0 && (
        <Button
          className="border-0 bg-transparent p-0 text-[11px] font-bold text-primary"
          type="button"
          onClick={() => void inspector.loadTurns()}
          disabled={inspector.state === "busy"}
        >
          Add turn separators
        </Button>
      )}
    </div>
  );
}

function SearchTab({
  inspector,
}: {
  inspector: ReturnType<typeof usePreviewInspector>;
}) {
  const form = useForm<OriginalSearchFormValues>({
    resolver: zodResolver(originalSearchFormSchema),
    defaultValues: { needle: "" },
    mode: "onChange",
  });
  const error = form.formState.errors.needle?.message;
  return (
    <form
      className="grid gap-3 border-0 pt-4"
      role="tabpanel"
      onSubmit={form.handleSubmit(
        (values) => void inspector.search(values.needle),
      )}
    >
      <p className="mb-3 text-[11px] leading-[1.5] text-muted-foreground">
        Search checks the original session locally and returns a count only. It
        never renders original text.
      </p>
      <div className="my-[18px] flex flex-wrap items-end gap-2.5">
        <div className="grid gap-1.5">
          <label htmlFor="original-search-needle">
            Search original session
          </label>
          <Input
            id="original-search-needle"
            {...form.register("needle")}
            placeholder="Client, hostname, token label…"
            aria-invalid={Boolean(error)}
            aria-describedby={error ? "original-search-error" : undefined}
          />
          <FormFieldError id="original-search-error" message={error} />
        </div>
        <Button
          className="rounded-[7px] border border-border bg-background px-[11px] py-2 text-[11px] font-bold text-foreground hover:border-primary hover:text-primary"
          type="submit"
          disabled={inspector.state === "busy" || !form.formState.isValid}
        >
          Check count
        </Button>
        {inspector.matches !== null && (
          <strong>
            {inspector.matches} original match
            {inspector.matches === 1 ? "" : "es"}
          </strong>
        )}
      </div>
    </form>
  );
}

function TurnTab({
  inspector,
  loaded,
}: {
  inspector: ReturnType<typeof usePreviewInspector>;
  loaded: boolean;
}) {
  return (
    <div className="grid gap-3 border-0 pt-4" role="tabpanel">
      {!loaded && (
        <p className="mt-[30px] mb-1 text-[13px] text-muted-foreground">
          Read transcript fully before loading turn index.
        </p>
      )}
      {loaded && inspector.turns.length === 0 && (
        <Button
          className="rounded-lg border-0 bg-primary px-3.5 py-2.5 text-[12px] font-bold text-primary-foreground hover:bg-primary/80"
          type="button"
          onClick={() => void inspector.loadTurns()}
          disabled={inspector.state === "busy"}
        >
          Load turn index
        </Button>
      )}
      {inspector.turns.length > 0 && (
        <div className="mt-5 grid gap-px border-t border-border">
          <span className="mb-3 block font-mono text-[10px] font-extrabold leading-none tracking-[.16em] text-primary">
            TURN INDEX
          </span>
          {inspector.turns.map((turn) => (
            <div key={turn.index}>
              <strong>
                {turn.index + 1}. {turn.role.replaceAll("_", " ")}
              </strong>
              <small>
                {turn.tool_name ?? "event"} · bytes {turn.byte_offset}–
                {turn.byte_offset + turn.byte_len}
              </small>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
