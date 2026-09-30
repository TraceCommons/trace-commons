type PageHeaderProps = {
  title: string;
  description: string;
  /** Accepted for the views the Monitor does not host; not rendered. */
  eyebrow?: string;
  /** Accepted for the views the Monitor does not host; not rendered. */
  phase?: string;
  /** Hide the title where a tab or breadcrumb already names the view. */
  titleHidden?: boolean;
};

/** Heading for a view inside a pane: title 17/600, one secondary line. */
export function PageHeader({
  title,
  description,
  titleHidden = false,
}: PageHeaderProps) {
  return (
    <header className="flex flex-col gap-1">
      <h1 className={titleHidden ? "sr-only" : "tc-heading"}>{title}</h1>
      {description ? (
        <p className="m-0 tc-label font-normal leading-[17px] tc-text-secondary">
          {description}
        </p>
      ) : null}
    </header>
  );
}
