type StatCardProps = {
  label: string;
  value: string;
  detail: string;
  tone?: "green" | "blue" | "gold";
};

export function StatCard({
  label,
  value,
  detail,
  tone = "green",
}: StatCardProps) {
  const toneClass =
    tone === "blue"
      ? "border-t-[3px] border-t-blue"
      : tone === "gold"
        ? "border-t-[3px] border-t-gold"
        : "border-t-[3px] border-t-green";

  return (
    <Card className={`min-h-[122px] border-t-2 ${toneClass}`}>
      <CardContent className="grid gap-1 p-5">
        <span className="text-xs text-muted-foreground">{label}</span>
        <strong className="font-heading text-2xl font-medium tracking-tight">
          {value}
        </strong>
        <small className="text-xs text-muted-foreground">{detail}</small>
      </CardContent>
    </Card>
  );
}
import { Card, CardContent } from "./ui/card";
