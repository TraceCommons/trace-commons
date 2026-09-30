import type { ReactNode } from "react";
import type { PreviewTurn } from "../api/preview-api";

type RedactedTranscriptProps = {
  body: string;
  turns: PreviewTurn[];
};

const redactionPattern = /<PRIVATE_[A-Z0-9_]+>|\[REDACTED[^\]]*\]/g;

export function RedactedTranscript({ body, turns }: RedactedTranscriptProps) {
  const segments = splitAtTurns(body, turns);
  return (
    <pre className="my-1 max-h-[430px] overflow-auto rounded-[9px] border border-border bg-muted p-4 font-mono text-[11px] leading-[1.55] text-foreground whitespace-pre-wrap leading-[1.7]">
      {segments.map((segment) => (
        <span key={segment.id}>
          {segment.kind === "turn" && (
            <span className="mt-2 block font-bold text-muted-foreground">
              — {segment.label} · turn {segment.turn} —{"\n"}
            </span>
          )}
          {segment.kind === "text" &&
            highlightRedactions(segment.text, segment.id)}
        </span>
      ))}
    </pre>
  );
}

type TranscriptSegment =
  | { id: string; kind: "turn"; label: string; turn: number }
  | { id: string; kind: "text"; text: string };

function splitAtTurns(body: string, turns: PreviewTurn[]): TranscriptSegment[] {
  if (turns.length === 0) return [{ id: "body", kind: "text", text: body }];
  const encoder = new TextEncoder();
  const segments: TranscriptSegment[] = [];
  let stringOffset = 0;
  turns.forEach((turn) => {
    const turnOffset = stringIndexAtByteOffset(body, turn.byte_offset, encoder);
    if (turnOffset > stringOffset)
      segments.push({
        id: `text-${stringOffset}-${turnOffset}`,
        kind: "text",
        text: body.slice(stringOffset, turnOffset),
      });
    segments.push({
      id: `turn-${turn.index}`,
      kind: "turn",
      label: turn.tool_name ? `tool: ${turn.tool_name}` : turn.role,
      turn: turn.index + 1,
    });
    stringOffset = turnOffset;
  });
  if (stringOffset < body.length)
    segments.push({
      id: `text-${stringOffset}-${body.length}`,
      kind: "text",
      text: body.slice(stringOffset),
    });
  return segments;
}

function stringIndexAtByteOffset(
  value: string,
  byteOffset: number,
  encoder: TextEncoder,
) {
  let bytes = 0;
  let index = 0;
  while (index < value.length && bytes < byteOffset) {
    const codePoint = value.codePointAt(index);
    if (codePoint === undefined) break;
    const character = String.fromCodePoint(codePoint);
    const characterBytes = encoder.encode(character).byteLength;
    if (bytes + characterBytes > byteOffset) break;
    bytes += characterBytes;
    index += character.length;
  }
  return index;
}

function highlightRedactions(text: string, keyPrefix: string): ReactNode[] {
  const parts: ReactNode[] = [];
  let lastIndex = 0;
  for (const match of text.matchAll(redactionPattern)) {
    const index = match.index ?? 0;
    if (index > lastIndex) parts.push(text.slice(lastIndex, index));
    parts.push(
      <mark
        className="rounded-[3px] bg-chart-4/20 px-1 font-bold text-foreground"
        key={`${keyPrefix}-${index}`}
      >
        {match[0]}
      </mark>,
    );
    lastIndex = index + match[0].length;
  }
  if (lastIndex < text.length) parts.push(text.slice(lastIndex));
  return parts;
}
