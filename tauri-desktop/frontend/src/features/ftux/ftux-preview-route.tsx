import { useNavigate, useSearchParams } from "react-router-dom";
import { MOCK_STORED_PASSKEY } from "./api/ftux-mock-data";
import { FtuxPage } from "./ftux-page";

// `#/ftux` opens the first-run flow on mock data. `?path=customize` starts
// on Customize and tailor; `?returning=1` opens as a returning user (P-7).
// It sits outside the real onboarding gate until the backend is wired, so
// finishing it changes nothing and returns to the app.
export function FtuxPreviewRoute() {
  const navigate = useNavigate();
  const [params] = useSearchParams();
  return (
    <FtuxPage
      initialPath={params.get("path") === "customize" ? "customize" : "connect"}
      returningPasskey={
        params.get("returning") === "1" ? MOCK_STORED_PASSKEY.name : null
      }
      onComplete={() => navigate("/", { replace: true })}
    />
  );
}
