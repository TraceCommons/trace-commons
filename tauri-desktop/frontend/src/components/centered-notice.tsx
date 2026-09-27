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
    <Alert variant={tone === "error" ? "destructive" : "default"} className="my-6">
      <AlertTitle>{title}</AlertTitle>
      <AlertDescription>{body}</AlertDescription>
    </Alert>
  );
}
import { Alert, AlertDescription, AlertTitle } from "./ui/alert";
