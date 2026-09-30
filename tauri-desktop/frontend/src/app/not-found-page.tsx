import { Link } from "react-router-dom";
import { buttonVariants } from "../components/ui/button";
import { routePaths } from "./routes";

export function NotFoundPage() {
  return (
    <section className="mx-auto grid max-w-[720px] gap-4 px-6 py-20">
      <span className="font-mono text-[10px] font-extrabold tracking-[.16em] text-primary">
        NOT FOUND
      </span>
      <h1 className="m-0">This page doesn’t exist.</h1>
      <p className="m-0 text-sm text-muted-foreground">
        Check the address or return to Insights.
      </p>
      <Link
        className={buttonVariants({ className: "w-fit" })}
        to={routePaths.insights}
      >
        Open Insights
      </Link>
    </section>
  );
}
