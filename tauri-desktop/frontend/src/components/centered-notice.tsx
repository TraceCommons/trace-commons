import { Notice } from "@/design-system";

type CenteredNoticeProps = {
  title: string;
  body: string;
  tone?: "neutral" | "error";
};

export function CenteredNotice({
  title,
  body,
  tone = "neutral",
}: CenteredNoticeProps) {
  return (
    <Notice
      tone={tone === "error" ? "outside" : "off"}
      title={title}
      className="my-6"
    >
      {body}
    </Notice>
  );
}
