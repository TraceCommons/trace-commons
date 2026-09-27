import { useWitnessCapacityNotice } from "../../../lib/tauri/use-contributor-copy";
import {
  nextRetryLine,
  type WitnessCapacity,
} from "../../../lib/tauri/witness-capacity";

// Approved sessions held because the privacy witness is busy. The words and
// the count come from the contributor core (`consent_copy`); only the retry
// time is rendered here, in local time.
export function WitnessCapacityNotice({
  capacity,
}: {
  capacity: WitnessCapacity;
}) {
  const copy = useWitnessCapacityNotice(
    capacity.waiting_sessions,
    capacity.wire,
  );
  if (!copy.data) {
    return (
      <div>
        <span role={copy.isError ? "alert" : undefined}>
          {copy.isError
            ? "Some approved sessions are waiting and have not been sent, but this build could not read the notice that says why."
            : "Loading…"}
        </span>
      </div>
    );
  }
  const next = nextRetryLine(copy.data, capacity);
  return (
    <div>
      <strong>{copy.data.title}</strong>
      <span>{copy.data.body}</span>
      {next && <small>{next}</small>}
    </div>
  );
}
