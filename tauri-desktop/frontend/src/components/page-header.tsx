type PageHeaderProps = {
  eyebrow: string;
  title: string;
  description: string;
  phase?: string;
};

export function PageHeader({
  eyebrow,
  title,
  description,
  phase,
}: PageHeaderProps) {
  return (
    <header className="mb-8 flex items-start justify-between gap-6 max-[767px]:flex-col">
      <div>
        <span className="mb-3 block font-mono text-xs font-semibold leading-none tracking-[.16em] text-primary">
          {eyebrow}
        </span>
        <h1 className="font-heading text-4xl font-medium tracking-tight sm:text-5xl">
          {title}
        </h1>
        <p className="max-w-2xl text-sm leading-relaxed text-muted-foreground">
          {description}
        </p>
      </div>
      {phase && <Badge variant="secondary">{phase}</Badge>}
    </header>
  );
}
import { Badge } from "./ui/badge";
