using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// What the window does for the controller: the two sharing sheets, and the
/// two reads Start makes between them and the commit.
/// </summary>
public interface IFirstRunSheets
{
    /// <summary>
    /// The scrub sheet, then the route sheet (SharingDisclosures.swift:104-205),
    /// acknowledging each on <paramref name="flow"/> as it is passed. False
    /// when the person cancelled either.
    /// </summary>
    Task<bool> ShowDisclosures(SharingDisclosureFlow flow, AutomaticGrantCopy grant);

    /// <summary><c>logged_in</c> from the daemon's status, read after the sheets.</summary>
    Task<bool> Connected();

    /// <summary>The core's answer to the progress (<c>tc_flow1_grant_request_json</c>), or null when unreadable.</summary>
    Task<Flow1GrantRequest?> GrantRequest(string progressJson);
}

/// <summary>
/// The first run's buttons, away from the window (checklist B5): every
/// click handler in FirstRunWindow calls one of these and then redraws,
/// so the decisions they make are tested here, through the runner they
/// drive (JoinScreen.swift, UsesScreen.swift, FirstRunFrame.swift).
/// </summary>
public sealed class FirstRunController
{
    private readonly IFirstRunSheets _sheets;
    private readonly Func<Task<AutomaticGrantCopy?>> _loadGrantCopy;
    private readonly UsesWords _uses;

    public FirstRunController(FirstRunRunner runner, IFirstRunSheets sheets, Func<Task<AutomaticGrantCopy?>> loadGrantCopy, UsesWords uses)
    {
        Runner = runner;
        _sheets = sheets;
        _loadGrantCopy = loadGrantCopy;
        _uses = uses;
    }

    public FirstRunRunner Runner { get; }

    /// <summary>The required use's name, once <c>consent_options</c> answered.</summary>
    public string? RequiredScope { get; set; }

    /// <summary>The terms document the Terms step shows, once fetched.</summary>
    public FetchedTerms? Terms { get; set; }

    /// <summary>The automatic contribution copy, once read (UsesScreen.swift:665-669).</summary>
    public AutomaticGrantCopy? GrantCopy { get; private set; }

    public bool GrantCopyRead { get; private set; }

    /// <summary>Start runs: the sheets, the core's answer, or the commit.</summary>
    public bool Starting { get; private set; }

    /// <summary>A refusal decided before a Start that then failed, said once one succeeds (UsesScreen.swift:275-309).</summary>
    public FirstRunFailure? PendingRefusal { get; private set; }

    /// <summary>The copy was read and would not decode: the Sharing card says so (UsesScreen.swift:705-709).</summary>
    public bool SharingUnavailable => GrantCopyRead && GrantCopy is null;

    /// <summary>The Sharing info button's words for the path in the state, or null for no button (UsesScreen.swift:700-703).</summary>
    public string? SharingDetail => UsesSharing.Detail(Runner.State.Sharing, GrantCopy, _uses);

    /// <summary>Read the copy once, as Uses and Join do when they appear.</summary>
    public async Task ReadGrantCopy()
    {
        if (GrantCopyRead)
        {
            return;
        }

        GrantCopy = await _loadGrantCopy();
        GrantCopyRead = true;
    }

    /// <summary>
    /// Back while the browser sign-in waits takes the person back, ending
    /// that sign-in as Cancel does (JoinScreenLayout.backWhileCommitting);
    /// any other commit withdraws Back.
    /// </summary>
    public bool BackWhileCommitting =>
        Runner.State.Step == FirstRunStep.Join && Runner.Committing == CommitPoint.SignIn && Runner.SignInWaiting;

    public bool BackEnabled => !Runner.IsCommitting || BackWhileCommitting;

    public bool CanContinue
    {
        get
        {
            FirstRunState state = Runner.State;
            if (Runner.IsCommitting)
            {
                return false;
            }

            return state.Step switch
            {
                FirstRunStep.Terms => Runner.HasAcceptedTerms(true, Terms?.Version, Terms?.ContentSha256),
                // Automatic goes through both sharing disclosures and the
                // core's answer when Start is pressed, as on the Mac. Start
                // is off until the chosen path's words are read whole
                // (UsesScreenLayout.canStart, UsesScreen.swift:100-108).
                FirstRunStep.Join => FirstRunNavigation.CanContinue(JoinStartState(state), RequiredScope) && !Starting
                    && UsesSharing.Available(state.Sharing, GrantCopy),
                _ => FirstRunNavigation.CanContinue(state, RequiredScope),
            };
        }
    }

    /// <summary>JoinScreenLayout.startState: the required use included, an account held read as enrolled.</summary>
    public FirstRunState JoinStartState(FirstRunState state)
    {
        FirstRunState ready = state.Clone();
        bool canForward = (ready.Account == AccountAnswer.NearAI || (ready.NearAIEnrolled && !ready.SignedOutOfEnrolment))
            && (ready.Account != AccountAnswer.NearAI || ready.SignedIn);
        if (canForward && ready.Account != AccountAnswer.NearAI)
        {
            ready.Account = AccountAnswer.Enrolled;
        }

        return ready.IncludingRequiredScope(RequiredScope);
    }

    /// <summary>The footer's Continue, Start on Join.</summary>
    public async Task Continue()
    {
        FirstRunState state = Runner.State;
        switch (state.Step)
        {
            case FirstRunStep.Tools:
                await Runner.Commit(CommitPoint.LeaveTools);
                break;
            case FirstRunStep.Terms:
                Runner.ContinueFromTerms(true, Terms?.Version, Terms?.ContentSha256);
                break;
            case FirstRunStep.Join:
                await Start();
                break;
            default:
                Runner.State = FirstRunNavigation.Next(state);
                break;
        }
    }

    /// <summary>The footer's Back: during the browser wait it ends the wait first.</summary>
    public async Task Back()
    {
        if (Runner.IsCommitting)
        {
            if (BackWhileCommitting)
            {
                await Runner.BackFromSignIn();
            }

            return;
        }

        Runner.State = FirstRunNavigation.Back(Runner.State);
    }

    /// <summary>A passed step on the thread.</summary>
    public void BackTo(FirstRunStep step)
    {
        if (!Runner.IsCommitting)
        {
            Runner.State = FirstRunNavigation.BackTo(step, Runner.State);
        }
    }

    /// <summary>A found tool's check (ToolsScreen.swift:589-700).</summary>
    public void AnswerTool(string kind, string path, bool on) =>
        Runner.State.Answer(kind, on ? ToolChoice.Watch(path) : ToolChoice.Off);

    /// <summary>One optional use's box on Uses.</summary>
    public void SetScope(string name, bool on)
    {
        if (on)
        {
            Runner.State.Scopes.Add(name);
        }
        else
        {
            Runner.State.Scopes.Remove(name);
        }
    }

    /// <summary>The person's pick of a Sharing choice; false when it was already the choice.</summary>
    public bool ChooseSharing(SharingPath path)
    {
        FirstRunState s = Runner.State;
        if (s.SharingChosen && s.Sharing == path)
        {
            return false;
        }

        s.Sharing = path;
        s.SharingChosen = true;
        return true;
    }

    /// <summary>The Terms step's Accept, for the document on screen.</summary>
    public void AcceptTerms(bool on, DateTimeOffset now)
    {
        Runner.AcceptedTerms = on && Terms is { } terms ? new AcceptedTerms(terms.Version, terms.ContentSha256, now) : null;
    }

    /// <summary>Join's Sign in: near.ai chosen and signed in at once (JoinScreen.swift:712-716).</summary>
    public async Task SignIn()
    {
        Runner.State.Account = AccountAnswer.NearAI;
        await Runner.Commit(CommitPoint.SignIn);
    }

    /// <summary>Join's Cancel while the browser sign-in waits.</summary>
    public Task CancelSignIn() => Runner.CancelSignIn();

    /// <summary>
    /// Start sharing (JoinScreen.swift:718-760): the required use included;
    /// on Automatic, the scrub and witness disclosures first, then the
    /// core's answer decides the state Start commits (Automatic with the
    /// grant ready, or Ask with the refusal said). Cancel on either sheet
    /// grants nothing and starts nothing.
    /// </summary>
    public async Task Start()
    {
        // Without the path's words nothing is shown or started: on the Mac
        // Start is off and no sheet can show (JoinScreen.swift:417-422).
        if (!UsesSharing.Available(Runner.State.Sharing, GrantCopy) || Starting || Runner.IsCommitting)
        {
            return;
        }

        Starting = true;
        try
        {
            Runner.State = JoinStartState(Runner.State);
            FirstRunFailure? refused = null;
            bool decidedNow = false;
            if (SharingDisclosureFlow.IsNeeded(Runner.State) && GrantCopy is { } grant)
            {
                var flow = new SharingDisclosureFlow();
                if (!await _sheets.ShowDisclosures(flow, grant))
                {
                    return;
                }

                bool connected = await _sheets.Connected();
                Flow1GrantRequest? request = await _sheets.GrantRequest(flow.ProgressJson(connected, Runner.State.Scopes));
                (FirstRunState next, refused) = SharingDisclosureFlow.Resolve(Runner.State, request);
                Runner.State = next;
                decidedNow = true;
            }

            FirstRunFailure? carried = decidedNow ? refused : PendingRefusal;
            await Runner.Commit(CommitPoint.Start, carried);
            if (Runner.Completed)
            {
                PendingRefusal = null;
                return;
            }

            // A Start that failed keeps the refusal for the retry; the
            // failure it stopped on is what the step says now.
            PendingRefusal = Runner.Failure is FirstRunFailure.CompleteFailed ? Runner.RefusedGrant ?? carried : carried;
        }
        finally
        {
            Starting = false;
        }
    }
}
