// INTEGRATION: link the production PrivateInferenceViewModel.cs and
// HarnessRowViewModel.cs into this test project beside its existing
// CredentialBrowser.cs link. The only replacements below are the daemon host
// and Windows browser boundary; copy, action tables and serializers use the
// actual Interop assembly and a freshly built contributor FFI library.
using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;
using TraceCommons.App;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests
{
    public sealed class PrivateInferenceProviderTests
    {
        private static readonly TimeSpan Deadline = TimeSpan.FromSeconds(3);

        [Theory]
        [InlineData(false)]
        [InlineData(true)]
        public async Task SelectedProviderSurvivesPendingStartAndRefusalAllowsRetry(bool requiresSession)
        {
            var recording = new RecordingCalls(requiresSession);
            var host = new DaemonHost(recording.CallAsync);
            var model = requiresSession
                ? new PrivateInferenceViewModel(host, requiresSession: true)
                : new PrivateInferenceViewModel(host);
            Assert.True(model.Available, "The current contributor FFI copy must be loaded.");
            await model.LoadCredentialAsync().WaitAsync(Deadline);

            // Onboarding receives an existing inference key with no session.
            // The requiresSession constructor must still offer sign-in.
            Assert.True(model.HasCredentialProvider);
            Assert.False(model.HasCloudSession);
            model.CredentialProvider = NearAiCredentialSurface.ProviderNear;
            Assert.True(model.HasCredentialWalletNotice);
            Assert.True(model.CredentialControlsEnabled);

            var enabledChanges = new List<bool>();
            model.PropertyChanged += (_, change) =>
            {
                if (change.PropertyName == nameof(model.CredentialControlsEnabled))
                {
                    enabledChanges.Add(model.CredentialControlsEnabled);
                }
            };

            Task<NearAiCredentialAttempt?> first = model.StartCredentialAsync();
            Assert.False(first.IsCompleted);
            Assert.False(model.CredentialControlsEnabled);
            AssertProvider(Assert.Single(recording.Starts), NearAiCredentialSurface.ProviderNear);

            model.CredentialProvider = NearAiCredentialSurface.ProviderGoogle;
            Assert.Equal(NearAiCredentialSurface.ProviderNear, model.CredentialProvider);
            Assert.Null(await model.StartCredentialAsync().WaitAsync(Deadline));
            Assert.Single(recording.Starts);
            Assert.False(first.IsCompleted);

            recording.FirstStart.SetResult(Refusal());
            Assert.Null(await first.WaitAsync(Deadline));
            Assert.True(model.CredentialControlsEnabled);
            Assert.True(model.HasNotice);
            Assert.Equal(new[] { false, true }, enabledChanges);

            model.CredentialProvider = NearAiCredentialSurface.ProviderGoogle;
            Assert.Equal(NearAiCredentialSurface.ProviderGoogle, model.CredentialProvider);
            Assert.False(model.HasCredentialWalletNotice);
            Task<NearAiCredentialAttempt?> retry = model.StartCredentialAsync();
            Assert.False(retry.IsCompleted);
            Assert.False(model.CredentialControlsEnabled);
            Assert.Equal(2, recording.Starts.Length);
            AssertProvider(recording.Starts[1], NearAiCredentialSurface.ProviderGoogle);

            recording.SecondStart.SetResult(DaemonResponse.Parse(
                "{\"result\":{\"attempt_id\":\"synthetic-retry\",\"status\":\"waiting_for_browser\","
                + "\"browser_url\":\"http://127.0.0.1:54321/near-ai/callback\"}}"));
            NearAiCredentialAttempt? attempt = await retry.WaitAsync(Deadline);
            NearAiCredentialAttempt started = Assert.IsType<NearAiCredentialAttempt>(attempt);
            Assert.Equal("synthetic-retry", started.AttemptId);
            Assert.True(model.CredentialControlsEnabled);
            Assert.False(model.HasNotice);
            Assert.False(model.HasCloudSession);
            Assert.Equal(new[] { false, true, false, true }, enabledChanges);

            // This exact sequence also rules out implicit enrollment, consent,
            // settings changes, and browser continuation after either reply.
            string[] expected = requiresSession
                ? new[] { DaemonProtocol.Methods.NearAiCredentialStatus,
                    DaemonProtocol.Methods.NearAiCredentialStart, DaemonProtocol.Methods.NearAiCredentialStart }
                : new[] { DaemonProtocol.Methods.NearAiCredentialStatus, DaemonProtocol.Methods.NearAiBalance,
                    DaemonProtocol.Methods.NearAiCredentialStart, DaemonProtocol.Methods.NearAiCredentialStart };
            Assert.Equal(expected, recording.Calls.Select(call => call.Method));
        }

        [Theory]
        [InlineData(false, "")]
        [InlineData(false, "NEAR")]
        [InlineData(false, "near\"}")]
        [InlineData(true, "")]
        [InlineData(true, "NEAR")]
        [InlineData(true, "near\"}")]
        public async Task InvalidSelectionCannotReplaceTheProviderSentToTheDaemon(bool requiresSession, string invalid)
        {
            var recording = new RecordingCalls(requiresSession);
            var host = new DaemonHost(recording.CallAsync);
            var model = requiresSession
                ? new PrivateInferenceViewModel(host, requiresSession: true)
                : new PrivateInferenceViewModel(host);
            Assert.True(model.Available, "The current contributor FFI copy must be loaded.");
            model.CredentialProvider = NearAiCredentialSurface.ProviderNear;
            model.CredentialProvider = invalid;

            Assert.Equal(NearAiCredentialSurface.ProviderNear, model.CredentialProvider);
            Assert.Empty(recording.Calls);
            Task<NearAiCredentialAttempt?> start = model.StartCredentialAsync();
            Assert.False(start.IsCompleted);
            AssertProvider(Assert.Single(recording.Starts), NearAiCredentialSurface.ProviderNear);
            recording.FirstStart.SetResult(Refusal());
            Assert.Null(await start.WaitAsync(Deadline));
            Assert.Single(recording.Calls);
            Assert.True(model.CredentialControlsEnabled);
        }

        private static void AssertProvider(RecordedCall call, string provider)
        {
            Assert.Equal(DaemonProtocol.Methods.NearAiCredentialStart, call.Method);
            using JsonDocument parameters = JsonDocument.Parse(call.Parameters);
            Assert.Single(parameters.RootElement.EnumerateObject());
            Assert.Equal(provider, parameters.RootElement.GetProperty("provider").GetString());
        }

        private static DaemonResponse Refusal() => DaemonResponse.Parse(
            "{\"error\":{\"code\":\"unavailable\",\"message\":\"synthetic-start-refused\"}}");

        private sealed record RecordedCall(string Method, string Parameters);

        private sealed class RecordingCalls(bool requiresSession)
        {
            public List<RecordedCall> Calls { get; } = new();
            public TaskCompletionSource<DaemonResponse> FirstStart { get; } =
                new(TaskCreationOptions.RunContinuationsAsynchronously);
            public TaskCompletionSource<DaemonResponse> SecondStart { get; } =
                new(TaskCreationOptions.RunContinuationsAsynchronously);
            public RecordedCall[] Starts => Calls
                .Where(call => call.Method == DaemonProtocol.Methods.NearAiCredentialStart).ToArray();

            public Task<DaemonResponse> CallAsync(string method, string parameters)
            {
                Calls.Add(new RecordedCall(method, parameters));
                return method switch
                {
                    DaemonProtocol.Methods.NearAiCredentialStatus => Task.FromResult(DaemonResponse.Parse(
                        requiresSession
                            ? "{\"result\":{\"state\":\"present\",\"session_state\":\"absent\"}}"
                            : "{\"result\":{\"state\":\"absent\",\"session_state\":\"absent\"}}")),
                    DaemonProtocol.Methods.NearAiBalance => Task.FromResult(DaemonResponse.Parse(
                        "{\"result\":{\"state\":\"absent\"}}")),
                    DaemonProtocol.Methods.NearAiCredentialStart when Starts.Length == 1 => FirstStart.Task,
                    DaemonProtocol.Methods.NearAiCredentialStart when Starts.Length == 2 => SecondStart.Task,
                    _ => throw new InvalidOperationException("Unexpected daemon call: " + method),
                };
            }
        }
    }
}

namespace TraceCommons.App
{
    // Test boundary for the linked production view model. Its public signature
    // matches DaemonHost.CallAsync, including optional parameters/cancellation.
    public sealed class DaemonHost(Func<string, string, Task<DaemonResponse>> call)
    {
        public bool IsRunning => true;
        public long ConnectionGeneration => 1;

        public Task<DaemonResponse> CallAsync(
            string method, string paramsJson = "{}", CancellationToken cancellationToken = default)
        {
            cancellationToken.ThrowIfCancellationRequested();
            return call(method, paramsJson);
        }
    }
}

namespace Windows.System
{
    // No test in this file authorizes opening a real browser.
    public static class Launcher
    {
        public static Task<bool> LaunchUriAsync(Uri uri) =>
            throw new InvalidOperationException("Unexpected browser launch in provider regression.");
    }
}
