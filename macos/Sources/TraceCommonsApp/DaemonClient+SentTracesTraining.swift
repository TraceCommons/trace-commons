import TCShellCore

// The two calls for adding Train coding models to sent traces
// (`model_training_consent_status`, `model_training_consent_grant`) go
// through `SentTracesTrainingClient` in TCShellCore, so their method names
// and parameters are asserted without a daemon. Calls block (the ABI is
// synchronous): callers run them off the main actor.
extension DaemonClient {
    var sentTracesTraining: SentTracesTrainingClient { SentTracesTrainingClient(transport: daemon) }
}
