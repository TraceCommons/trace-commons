# Paxel's actual flow (read from the Notion page, 2026-08-27)

paxel.ycombinator.com is geo-blocked ("Paxel isn't available in your country
or region") from a real Chrome session, not just from a fetch. But Devfolio
embedded the steps in the feedback page itself, inside a toggle that Notion's
text extraction collapses. Read visually.

## Option 1: All my repos (Recommended)

"Best for a broader picture across projects. Change into the parent folder
that holds your repos, then run."

    curl -fsSL https://paxel.ycombinator.com/upload.sh | bash

## Option 2: Just one repo

"Best for focusing on a single project. Change into that project's folder
(replace ~/path/to/your-project with the real path) and run."

    cd ~/path/to/your-project && curl -fsSL https://paxel.ycombinator.com/upload.sh | bash

## Their discovery aid

"You can use this prompt in Claude, Codex, or Cursor to find all your repos on
your machine with AI transcripts, show you the list, and hand back ready-to-run
commands for the ones you pick."

    Find every repo on my machine where I've used Claude Code, Codex CLI,
    or Cursor. Check ~/.claude/projects/, ~/.codex/sessions/, and ~/
    Library/Application Support/Cursor/User/workspaceStorage/ (macOS).

    For each repo, list name, absolute path, and total session count. Ask
    me which ones to include.

    For each one I pick, hand back this command with the path filled in:

    cd <ABSOLUTE_PATH> && curl -fsSL https://paxel.ycombinator.com/upload.sh | bash

## What this actually tells us

1. **There is no installation.** The flow Devfolio praised is `curl | bash`
   run directly. No binary on PATH, no persistent state, no second command.
   "I liked their installation flow" means "there was nothing to install."
   Our answer to that is the one-time script (Slice F), not install.sh.

2. **It is one command, not two.** Option 1 and Option 2 are the same command;
   only the working directory differs. The feedback's "two commands" is one
   command run from two places.

3. **Scoping is by directory subtree, not by time.** Option 1 is "cd to the
   parent folder that holds your repos". Our bare `submit` scopes by a 7-day
   window across all projects, which is a different selection entirely. A
   hacker running ours from ~/code gets "everything everywhere in 7 days",
   not "the repos under here".

4. **Paxel reads Cursor** (~/Library/Application Support/Cursor/User/
   workspaceStorage/) alongside Claude Code and Codex. Relevant to item 1's
   "the more agents your CLI can support".

5. **No verification at all.** `curl | bash` against an unsigned script. Our
   install.sh deliberately refuses that posture, and this is the one place we
   should NOT follow Paxel. Note the asymmetry is defensible: Paxel installs
   nothing persistent, we install a binary that reads coding transcripts.
