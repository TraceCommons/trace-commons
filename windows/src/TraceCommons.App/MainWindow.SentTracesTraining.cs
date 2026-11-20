using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using TraceCommons.App.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App;

/// <summary>
/// Train coding models on traces already sent (W20-D3, taken (a)): the one
/// model the app owns (<c>AppModel.sentTracesTraining</c>,
/// AppModel.swift:362-367), so the Settings sheet, Home's line and the
/// inspector's section agree. Raised where the Mac raises it: once after a
/// confirmed turn-on in Settings (AppModel.swift:3007-3017), and by Home's
/// Review. Nothing is preselected and no grant is sent without Allow.
/// </summary>
public sealed partial class MainWindow
{
    private SentTracesTrainingModel? _sentTraining;
    private SkillLearningModel? _skillLearning;

    /// <summary>
    /// The tested-skill flow's one model (W20-D2, taken (a)): per-session
    /// state the History inspector draws (<c>AppModel.skillLearningStore</c>).
    /// </summary>
    private SkillLearningModel SkillLearning =>
        _skillLearning ??= SkillLearningModel.WithCore(SkillLearningCall());

    /// <summary>
    /// The daemon the skill flow asks, or none in a sample window: the flow
    /// spends on NEAR AI and writes into a tool's folder, and a recorded
    /// sample answers none of it (Program.IsSampleLaunch).
    /// </summary>
    private System.Func<string, string, Task<DaemonResponse>>? SkillLearningCall()
    {
#if DEBUG
        // The skillLearning extra answers the flow from fixtures, so its
        // stages can be drawn; it records no write.
        if (_host.IsSample && !_host.SampleAnswersSkillLearning)
        {
            return null;
        }
#endif
        return (method, parameters) => _host.CallAsync(method, parameters);
    }

    /// <summary>The model, made on first use over this window's daemon.</summary>
    private SentTracesTrainingModel SentTraining =>
        _sentTraining ??= new SentTracesTrainingModel((method, parameters) => _host.CallAsync(method, parameters), SentTracesTrainingCopy.Current);

    /// <summary>Re-reads the count with Home's reads and redraws the line (SentTracesTrainingViews.swift:119-126).</summary>
    private async Task RefreshSentTrainingLineAsync()
    {
        await SentTraining.RefreshAsync();
        _home.SetTrainingLine(SentTracesTrainingViews.Line(SentTraining, ReviewSentTraining));
    }

    /// <summary>Home's Review: the same sheet, in this window.</summary>
    private void ReviewSentTraining()
    {
        SentTraining.Review();
        if (Content?.XamlRoot is XamlRoot root)
        {
            _ = ShowSentTrainingSheetAsync(root, SentTracesTrainingSite.Monitor);
        }
    }

    private async Task ShowSentTrainingSheetAsync(XamlRoot root, SentTracesTrainingSite site)
    {
        await SentTracesTrainingViews.ShowSheetAsync(root, SentTraining, site);
        _home.SetTrainingLine(SentTracesTrainingViews.Line(SentTraining, ReviewSentTraining));
        _historyDetail?.RefreshTraining();
    }

    /// <summary>
    /// A Settings consent write the daemon confirmed: when it turned Train
    /// coding models on, read the eligible traces and raise the sheet once,
    /// in the Settings window (<c>offerSentTracesTraining</c>, AppModel.swift:3007-3017).
    /// </summary>
    private async void OnConsentScopesConfirmed(IReadOnlyList<string> before, IReadOnlyList<string> confirmed, XamlRoot? root)
    {
        if (!SentTracesTrainingState.TurnsOn(before, confirmed) || root is null)
        {
            return;
        }

        await SentTraining.OfferAfterTurnOnAsync(SentTracesTrainingSite.Settings);
        await ShowSentTrainingSheetAsync(root, SentTracesTrainingSite.Settings);
    }
}
