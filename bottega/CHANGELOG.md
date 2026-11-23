# Changelog

All notable changes to Droppy Code are documented here, newest first. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions are the release tags without the `v`.

`scripts/publish_release.sh` reads the section whose heading matches the version in `project.yml` (for example `## [1.5.4] - 2026-09-17`) and publishes it as the GitLab release notes, which Settings › About shows as its New features / Bug fixes / Refinements cards. `scripts/build_changelog.py` builds `website/changelog.json` for the site from the same sections. Work that has merged but not shipped sits under `## [Unreleased]`, which both scripts skip; the next release renames that heading to its version and date. A `### Thanks` heading under a version names every outside contributor whose merge request shipped in it, in the form `- Name (@gitlab-handle): what they did (!MR).`; `python3 scripts/release_credits.py` prints the block for the changes since the previous tag, and publishing stops until every contributor is named.

## [Unreleased]

### Bug fixes
- A framework's own raise while a notification is posted no longer ends the app: on macOS 27 a popover's window coming on screen can make the framework's remote view throw from inside the notification that announces it, and because that happens during a SwiftUI layout pass AppKit catches the exception and stops the app, which is how both reports of September 18 read, both from the release-notes cards on Settings › About. The app's notification posts now catch such a raise, log it with the raising observer's frames, and carry on, so the popover's window ordering and the popover itself still finish.
- The New features, Bug fixes and Refinements cards on Settings › About open their section in a card under the row instead of in a popover: the notes read in the page and scroll with it, and tapping the card again, or the card's close button, folds them away.
- The Codex usage panel names the ChatGPT plan the way OpenAI sells it: the `pro` account type is the $200 Pro tier and now reads "Pro 20x" where the panel showed "Pro", and `prolite` reads "Pro 5x" where it showed "Prolite"; plan types only Codex spells get its own CLI wording, so the self-serve business Pro Lite entitlement reads "Business Premium" instead of "self_serve_business_prolite".

## [1.6.0] - 2026-09-18

Droppy Code 1.6.0 folds a finished turn into its "Worked for …" line with the turn's steps behind the chevron, keeps a running turn's steps, and the agent's thinking, in the card below its bar, hands a chat stopped by its provider's usage limit a New thread that carries the work on, lets the heads leave the sidebar on their own, and puts a mark of your choosing in front of a project's name; Netlify joins the MCP catalog. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Netlify joins the MCP catalog: sign in with your Netlify account and a thread can create projects, deploy sites and manage environment variables.
- Projects wear a mark: each project can be given an emoji or an SF Symbol in Settings, General, Projects, and the activity list can draw that mark in front of a thread's name in place of the line that named the project, one line per thread, while what the thread is doing still reads at the end of the row. The choice of list sits on the General page and on the welcome screen, and a new install starts on the compact one.
- A chat that stops on its provider's usage limit now leaves a notice with two ways out beside it: Cancel stops the automatic continue and leaves the chat to you, and New thread stops the wait and opens a new chat of the same project holding a hand-off of the request and of what the stopped turn had already done, so another model can be picked there and sent it.
- Heads can leave the sidebar by themselves: with Auto-hide heads in the sidebar on (Settings, Hydra, Heads), a chat's head card shows while one of them runs and goes once every head of that chat has finished, coming back on its own when one starts again. Off by default, so heads stay where they are.

### Bug fixes
- The GitLab MCP server no longer looks like a hiccup on GitLab's side: a top-level group that hasn't allowed MCP client access is answered by GitLab 19.4 and earlier with 404 instead of "MCP server disabled", so connecting ended in "gitlab.com didn't answer its MCP address (404), try again in a few minutes". That answer now opens the steps to turn MCP on for the group, and the app's own MCP proxy answers a path it has no route for with 502 rather than a 404 that reads as the server's reply.
- Scrolling through the chat and its panels no longer fights the reader: the timeline reads wheel ticks and a viewport that changes height as the reader's own movement rather than as content arriving, so rows landing or the composer growing never tugs anyone who is still scrolling, and the Settings pages settle a scroll burst without re-evaluating the page on every frame.
- The diff panel builds a file's rows once per change and drops a scroll request a newer one has replaced, the model list keeps its measured row heights without re-evaluating the page for every measurement, a pre-measured popover lays its content out once instead of on every parent update, the update notes on the About page scroll with the page instead of inside a scroller of their own, and the shortcuts page lays out whole so its scroller knows its content height.

### Refinements
- A finished turn reads folded: it shows its final reply with the turn's plans and any warning, and the steps it took, its heads and its changed-files card wait behind the "Worked for …" line, which opens them in a card of their own.
- A running turn keeps its work on the working line: the progress card stays the card it is, and the steps the turn is taking, with the agent's thinking and what it says between them, open under it once its chevron is clicked; the reply that concludes the turn lands in the chat once the turn ends.
- The Chronological order switch is gone from Settings › Conversation: a turn reads folded until its line is clicked, which is what the app always does now, so the switch had nothing left to change.

### Thanks
- René (@diazdesandi): confined `search_text` to the project root with a `--` guard against flag smuggling, kept plan mode blocking edits after approve-for-turn, cleared the save timer handle before `saveNow` bails and stopped `WorkingTreeWatch` tearing down the registered watch when a second caller lost the race (!307); one OpenAI-compatible session and one Keychain behind the DeepSeek, Meta and Z.ai providers (!308).
- Bryan Galdámez (@JosueGalRe): finished turns open chronologically and a head wears its profile (!283); native heads routed to head profiles on Claude, Codex and Copilot (!285); the pair editor keeps its effort row for a model whose catalog entry declares no efforts yet (!304); a model catalog probe that came back empty is tried again on the next ask instead of closing the provider's list for the rest of the run (!306).
- Nikita Tsyganov (@nik1tsyganov): named the parts of the Pi usage sum so it type-checks (!293).

## [1.5.6] - 2026-09-17

Droppy Code 1.5.6 stops a new chat from being told its agent had lost a conversation that never was, and draws the uncapped Heads at once choice as an infinity sign. Apple silicon, macOS 26 or later. Signed and notarized.

### Bug fixes
- A first message no longer opens with "Claude no longer has this conversation. It starts a new one here": the session started while the message was being typed was resumed as a conversation the CLI had never written whenever a setting changed the launch in between (effort, fast mode, the heads' provider coming online, a heads cap). A session's ID now reaches the chat with its first message, and a session replaced before then is followed by one that starts fresh, with no resume attempt and no warning.

### Refinements
- The Heads at once row on the Hydra page shows the uncapped choice as an infinity sign instead of the word Off; it reads as "No cap" to VoiceOver, the wording a pair's own editor uses.

## [1.5.5] - 2026-09-17

Droppy Code 1.5.5 gives a Hydra pair more to work with: named head profiles that route each task to a purpose-built configuration, a global cap on how many heads work at once, custom pair names with fused icons, and a running turn that can stay in the order it arrived. It also fixes resumed Claude Code chats the CLI no longer remembers, token totals for resumed chats and cross-project heads, and merges that a head's build output had swamped. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- A pair can carry head profiles: purpose-named configurations ("quick", "deep", "visual" or any word) with their own provider, model or effort, edited in the pair's Head profiles section in Settings. The lead hears which profiles exist and routes each task to one by name in its delegation block; a task without a profile, or with a name the pair does not have, runs on the heads' shared model as before.
- Chronological order, a switch in Settings › Conversation: a running turn's replies and tool runs stay in the order they arrived instead of the steps before the last tool call gathering into the working line. Off by default; finished turns keep their fold either way.
- Heads at once, a row of buttons on the Hydra page: cap how many heads work at a time for every chat, Off or 1 to 8, without setting up a pair. Off by default; a pair with a lower cap of its own keeps it.
- A pair can carry a name of its own, typed on the Hydra page in Settings, and shows as a fused icon of its lead's and heads' provider marks in the composer's model picker and model chip.

### Bug fixes
- Opening the working line while scrolled up now follows the stream: the timeline pins to the end and keeps up with the reasoning as it arrives, instead of leaving the expanded thinking fixed while the text streams out of view below.
- OpenCode models show their reasoning levels before a chat starts: the effort slider no longer says "This model has one reasoning level" for a model that has several until the first message is sent, and starting a chat no longer forgets the levels of every other OpenCode model. OpenCode reports the levels of the model a session is on, so the catalog now asks it for each model in turn, the way it already did for Cursor.
- Resuming a Claude Code chat whose conversation the CLI no longer has ("No conversation found with session ID") starts the session over instead of leaving the chat stuck: a CLI that exits before answering the first request is treated as a lost session and a new one begins.
- Token totals are right again for resumed chats: a resumed Codex chat no longer adds the whole conversation's total on every turn, Claude Code usage is counted per turn, and heads working in another project count toward their chat.
- A merge no longer carries a head's build output: files under build folders that a head's command touched are left out of the merge and its command rows, so a merge that succeeded is no longer reported as one that did not.
- Heads sent to another project now form that project's merge: their landed files were reported as outside every project because the paths were never anchored to that checkout.
- The Hydra reports popover no longer jumps when a head's report is opened: expanding a report no longer animates the toggle the way the popover's height is measured, and the report lays out eagerly instead of lazily, so the popover takes its height once rather than resizing a second time after the rows appear.
- Settling a chat with the Settled section folded no longer leaves its row fading out in the empty space below the header: the row's ghost follows the header as the list closes up and is absorbed into it, and a ghost leaving past the list's edge slips out without fading on the way.

### Refinements
- The disk image opens to a styled drag-to-Applications window, with its own background and the app icon as the volume icon, the same installer window Droppy has.
- The merge card in the timeline now walks a track of dots, one per stage, that fills as the merge moves through its stages, with the head hopping along above it and giving a small bounce once every dot lights; finished heads and the merge pill settle into place with a glide transition.
- Head popovers now open on the head itself when a head's name or mention is clicked in the timeline: a finished head's pill opens its report under a header with the head's glyph, name, status and task, and a brief's pill opens the brief the same way, while the delegation card names each head it summons with its glyph and the project it goes to.

## [1.5.4] - 2026-09-17

Droppy Code 1.5.4 gets a Hydra team moving sooner: heads start the moment their brief is written rather than when the lead's whole reply is done, a lead can send heads to any project in the sidebar, and an older Claude Code no longer fails to launch when Hydra is on. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Heads start while the lead is still writing: each one goes out the moment its entry in the delegation block is complete, so the first head is working before the last brief is typed.
- A lead can send a head to another project in the sidebar, or to any git repository on the Mac by path, which then joins the sidebar. Each project's work lands and merges on its own.

### Bug fixes
- With Hydra on, a Claude Code older than the flags Droppy Code passes it no longer fails at once with "unknown option": the app checks what the installed CLI understands and passes only that.
- The app no longer aborts when a window's chrome is redrawn in the middle of macOS laying out the title bar.
- A merge whose files were already on the default branch, taken along by another chat or by hand, no longer posts a card about it: the merging pill simply goes.
- Hydra's merge summary no longer lists the build output, ignored paths and a sibling chat's files it left out, which read as if something had gone wrong.

### Refinements
- The picture tiles in Settings (sidebar layout, heads' panels, working line, checkouts) are redrawn as crisp little windows that fill their tile, and the part each option changes lights up in accent on the chosen one.
- The source is arranged in four layers, Core, Services, UI and App, so a change to a model, a provider or a view has one home.


## [1.5.3] - 2026-09-17

Droppy Code 1.5.3 puts the keyboard in charge: Escape stops a turn and takes it back, ⌘B hides the sidebar, ⌘1 to ⌘9 switch threads, a drag selects text across a whole reply, and a large Hydra team no longer freezes the app. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- A drag selects text across a finished reply the way it does in any Mac text view: headings, paragraphs and lists are one run of native text, so the selection runs from one paragraph into the next and ⌘C copies it. Code blocks and tables keep their own views between the runs, and links still open on a click.
- Escape on a running turn stops it and takes it back: the conversation and the provider's context rewind to before it, and the message returns to the chat box to be changed and sent again. Files stay as the turn left them.
- Escape dismisses whatever is in front: a popover, a panel, a sheet, the terminal's search. A field being typed in keeps its Escape.
- ⌘B shows and hides the sidebar.
- ⌘1 to ⌘9 switch to the first nine threads, before the menu bar sees the chord.
- ⌘W archives the thread, and the question names it: Return archives, Escape keeps it, and Delete sits beside Archive.

### Bug fixes
- Twenty heads out at once no longer freeze the app: heads pace their flushes by how many are streaming, at most eight run at a time with the rest queued, and the progress bar, the Hydra button and the delegation card stopped doing work on every frame.
- Streamed text no longer gets stuck fading in when a reply is rewritten shorter, or when a character spans two segments of the veil.
- Dragging a floating panel with several open no longer stutters.
- A scroll requested while the timeline lays out no longer loops until macOS kills the app.
- A terminal that just opened takes the keyboard, so typing goes to the shell rather than the chat box.
- The usage popover no longer ends on a divider when a plan has no resets to bank.
- The heads' report popover, a head's steps popover and the plan popover open as tall as what they hold, up to a cap, instead of collapsing to a few rows with a scrollbar.

### Refinements
- The Sidebar and Heads' panels pickers in Settings show what each choice looks like: a tiny window with its column or floating panel, and one team panel against a panel for every head. The Hydra picker's options are now "One panel" and "A panel each".
- A thread opens on its last three blocks, so switching to a long chat paints at once.
- The chat columns either side of the selected thread stay built, so the arrow keys and the next click open them without a rebuild.
- Landing a head's work skips the build output when it makes its patch, so large teams merge faster.


## [1.5.2] - 2026-09-17

Droppy Code 1.5.2 fixes a crash the moment a lead's heads went out, and makes switching between chats with and without floating panels land in one step instead of gliding and jumping. Apple silicon, macOS 26 or later. Signed and notarized.

### Bug fixes
- The app no longer dies when a reply is rewritten shorter while its newest text is still fading in, which happened the moment a lead's heads went out and the delegation block left its reply.
- Switching to a chat with a different set of floating panels no longer glides the conversation, fades the panels and shifts the chat box all at once: the column snaps to the new chat's layout in one step, and a team panel held over from the previous chat no longer lingers on the new one.


## [1.5.1] - 2026-09-17

Droppy Code 1.5.1 fixes Hydra merges from a shared checkout so each chat merges only what its own team changed and says so when there is nothing left to merge, lets you drag a chosen model all the way to the top of the list, and turns rows of toggles in Settings into picker tiles. Apple silicon, macOS 26 or later. Signed and notarized.

### Bug fixes
- Hydra's automatic merge no longer takes along files another chat's team landed in the same checkout: each chat's merge carries only its own team's work, and the other chat's merge then still has its files to land.
- When a chat's work already sits on main (an earlier merge took it along, or it was committed by hand), Hydra says so in the chat and marks the heads merged, instead of ending without a word.
- When the team's files lie outside the chat's project, Hydra names the project they belong to and asks you to open the chat there, instead of merging nothing silently.
- Reordering your chosen models in Settings: a row no longer loses its drag or changes height mid-reorder, so it can be dragged all the way to the top of the list.

### Refinements
- Rows of related toggles in Settings became picker tiles with a small preview each: workspace, window and sidebar choices in General, the sidebar mode, and the Hydra head rows.
- Every head named anywhere in a reply gets its dragon and colour, not only the one that opens the sentence, links included; the heads' reports popover shows each head on its own card with a green done mark.
- Every badge and card in the conversation uses one pill style, with the same metrics throughout.
- The card that lists what each head is being sent out to do stays on screen a moment once the heads are out, wearing the same wave the heads panel shows for a finished head, and then fades away, instead of vanishing the instant the briefs are written.
- The lead is told that heads and the merge only see the chat's own project, so it says so and asks you to open the chat there rather than working in another repository unmerged.


## [1.5.0] - 2026-09-16

Droppy Code 1.5.0 fades streamed text in as it arrives, keeps a running turn's steps inside its working card, starts a project's new chats on the pair you choose, and finds OpenCode's newest models without a relaunch. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Right-click a project in the sidebar to choose the Hydra pair each of its new chats starts on; "The chat's last pair" removes the rule, and a pair removed from Settings takes its project rules with it. Contributed by nik1tsyganov.

### Bug fixes
- The app no longer dies when a layout pass asks for one redraw too many while macOS works out the window's drag region; the request is taken again a moment later, whichever view made it. 1.4.1 guarded one view, and the crash moved to the scroll view beneath it.
- The Providers page refresh reloads the model lists of OpenCode, Cursor, Grok and Devin too, so a model OpenCode gained after launch, such as Union Alpha, is findable in Settings without relaunching the app.

### Refinements
- Text streaming into a reply fades in where it lands instead of popping in whole, the fade keeping pace with the stream; nothing moves or resizes while you read.
- A running turn keeps every step, and what the agent says between steps, in its working card until the turn ends, so earlier steps no longer sit above the card as a collapsed line and the timeline reads the same while running and once folded.
- File attachments fill their chip with the file's icon, the way photos do, instead of a small icon over a truncated name.


## [1.4.1] - 2026-09-16

Droppy Code 1.4.1 folds every finished turn into its summary, keeps the app up through a layout crash, and makes Hydra heads on Z.ai think at the working effort they were promised. Apple silicon, macOS 26 or later. Signed and notarized.

### Bug fixes
- A turn the app quit under folds into its "Worked for" summary when the chat is reopened, like every other finished turn; it stayed open with every step spilled into the chat.
- The app no longer dies when macOS works out the window's drag region in the middle of a layout pass; the refused redraw is taken again a moment later.
- With "Heads work at a working effort" on, a lead at Max on a model whose scale has no medium (Z.ai's GLM) sends its heads out at the scale's middle rung; they went out at Max.
- Head pills in the chat share one left edge: the narrower pill hung off the widest one's right edge, and each head that finished shifted the others.
- The turn's diff counts only the files the chat itself changed, from the tool edits and the provider's own patch together; a file edited elsewhere in the repository meanwhile no longer lands in the turn's summary.
- A command that wrote files counts as an edit on a head's progress bar, the way its row already counts it.

### Refinements
- The head progress bars cache their staves between frames instead of walking every step of the timeline thirty times a second; three hangs of six seconds each came from that walk.
- A question from the agent, or the chat box growing under the last message, brings the conversation's end back into view; a new row while you are reading further up leaves you there.
- Banked resets sit on one line, "Banked resets · 2 available", with the outcome of a spend on a line below it.
- The diff view keeps its line numbers in fixed columns, so wrapped code stays in its own column instead of running back under the numbers.
- The clear button in the search field keeps its room when the field is empty, so the text no longer shifts when you start typing.
- Text in a reply is selected within a paragraph again; the whole-reply drag mode from 1.4.0 is gone.


## [1.4.0] - 2026-09-16

Droppy Code 1.4.0 brings Z.ai's reset cards into the usage popover and lets a drag select across a whole reply. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Banked resets for Z.ai: the usage popover and Settings, Providers show the reset cards a Coding Plan account holds, for the 5-hour and the weekly limit, each with its expiry; spend one, after a confirm step, and its limit goes back to zero.

### Refinements
- Selecting text in a reply just works: drag anywhere on a finished reply and the selection runs across the whole of it, paragraphs, lists, code and links alike; links still open on a click. The right-click Select text item is gone, since the drag does its job.


## [1.3.1] - 2026-09-16

Droppy Code 1.3.1 puts the head card in the merge popover where it belongs, shows reply quotes on a sent message as the chips they were in the chat box, and gives the Z.ai mark its colour. Apple silicon, macOS 26 or later. Signed and notarized.

### Bug fixes
- Hovering a head in the merge popover opens its card from that head's face: the card hung from the middle of the row, between the faces and the count, whichever head was under the cursor.
- A message sent with reply quotes shows them above the bubble as the chips the chat box showed them in, and the bubble keeps only the typed words; the quotes came through as raw quote lines.
- The Z.ai mark takes the label colour like the other provider marks; it drew black on a dark card.


## [1.3.0] - 2026-09-16

Droppy Code 1.3.0 brings Hydra its cookbook: ready-made lead-and-heads pairs in Settings, an intro the first time Hydra is switched on, and popovers that show each merge and report as it lands, alongside Pi and Z.ai joining the providers, banked resets for Codex, slash commands and reply quotes in the composer, and follow-ups that bundle into one message while a turn runs. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Pi joins the providers, with supervised, auto-accept and full-access approval levels.
- Slash commands in the composer: type / to lead a message with a command.
- Reply quotes: quote part of a reply to answer it in place.
- Hydra pair recipes and the cookbook page in Settings: ready-made lead-and-heads pairs with an effort for each side, and providers fall back to what is signed in.
- A Hydra intro popover the first time Hydra is switched on.
- The Hydra merge and report popovers: merge request notes with their files and status, and report digests with each head's outcome and landed files.
- Follow-ups sent while a turn runs are bundled into one message.
- Finished heads pop into panels on their own.
- The last Hydra pair is remembered and applied to new chats.
- A Discord invite button.
- Z.ai joins the providers, with the GLM Coding Plan, GLM models and plan limits in the usage popover.
- Banked resets for Codex: the usage popover and Settings, Providers show the resets a Codex account has banked; spend one, after a confirm step, to clear its active windows, and the bars spring back down.
- Command-W asks to archive the thread, Command-S settles it.
- Questions with several to answer come one per page, with navigation between them.

### Bug fixes
- A running chat no longer shows nothing until it is reopened: the timeline could take content growing under it for a scroll still coasting and stop following, and a Hydra note landing mid-turn split the turn into two blocks under one name, leaving the running tail undrawn.
- A thread reopened at its top: when its first rows fit the pane, the history landing above left the view on the oldest rows; it is put back on the end.
- A thread stopped mid-turn (settled, archived, its pair switched) closes its turn instead of showing the working line for good; deleting a running head tells its lead; Stop no longer waits on git for its diffs.
- Escape in Settings or a file picker no longer closes a chat popover underneath it.
- Hydra on every provider: heads run with full access in their own copy, so none waits on a permission prompt nobody can answer; Codex heads are recognised however the app-server announces them; a Claude head's stream survives the lead's turn ending; a block anywhere in a reply sends its heads, a block on a stopped turn says so, and a pair whose heads' provider is off says where the heads run instead; an Antigravity head starved of permissions reports a failure rather than a clean finish.
- The effort slider runs from low to high on Grok: its levels arrived from extra high down, and the knob sat at Low for the highest.
- The freeze after typing caused by the key view loop is gone, AppKit owns the loop now.
- The merging row's stage words no longer tear mid-transition.
- The working badge's time no longer jumps out of the badge when the steps open.
- Sidebar hover tracking and file card spacing are fixed.
- Nested links in Hydra report rows no longer steal the button's click.
- A link inside a merge note opens the merge request.
- Selectable text no longer overflows the row below it: it is framed to its measured height.
- Opening a work group, step or tool scrolls it into view, and copying a link confirms with a checkmark.
- Heads announced only through activity notes still join the team, and spawning notes read more clearly without the lead repeating each head's status.
- The merge pill no longer shows a stale elapsed time once the merge has landed.
- A thread keeps its model as the live catalogue changes: the built-in list holds the reference while the live list refines the details.
- An interrupted turn settles instead of hanging, and the Hydra chevron only shows where there is something to open.

### Refinements
- Floating panels dock on both sides of the chat only when both fit beside it; dragging and edge-resizing a panel no longer re-lays out the chat every frame, and sidebar and terminal drags step and settle like a window resize.
- The merge popover shows the heads whose work went into the merge; hovering a head names its task and the files of its that landed.
- Head reports open in a plain popover with a pinned header and a compact body; a head's provider mark always shows in its panel; the team's rows sit on the agent's side of the chat; a head going out is announced with its own face rather than a spinner.
- The welcome tour has eight pages of new captures over the desktop, with the floating thread list and Hydra's cookbook among them.
- Chat zoom lives in Settings with a live preview.
- History loads asynchronously so switching chats is instant.
- The app launches straight into the chat.
- The model chip and thread rows use right-click popovers.
- While a turn runs, the working line is the head panel's progress card: the task breathing, the stave bar filling with the turn's steps, the time in its pill, and the chevron opening the steps; Settings › Conversation › Show activity while working turns it back into the plain line.
- The question popover opens on its badge at one size and stays put while you answer; it is plain and native now.
- The checklist card hugs its steps like the other badges.
- Hydra recipes name current models with flexible effort levels.
- The license is GNU AGPL v3 with attribution.
- Cursor's reasoning levels come through a parameterized model picker.
- A Debug build runs as Droppy Code Dev beside the release app, with its own library and settings.
- Chat switching stays instant on long threads: a turn is finished once, a thread is written once, and snapshots and change lists cost less per turn.
- Commands stop with their turn, provider sessions stop cleanly, and the composer and message editor render more cheaply.
- The changelog page on the site is redesigned, its picture tiles take a top-copy, bottom-picture layout, and the composer tour gains scenes for notifications, commands and quotes.


## [1.2.0] - 2026-09-15

Droppy Code 1.2.0 turns one chat into a team: switch Hydra on and the agent leads helper heads on big jobs, each in a copy of your project, their work landing back in your checkout as one merge. Command Code joins the providers, threads settle instead of archiving, a welcome tour opens on first launch, and the usage popover shows every limit you are paying for. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Hydra: one chat, many heads. The lead writes the briefs, sends heads out in parallel, hears each report back in the chat, and lands the team's work in your checkout as one merge; a switch in the model chip's right-click popover and a Hydra page in Settings.
- Pairs in the model picker: a pair sets the lead's model and the heads' model, and can cross providers, a Claude lead over Gemini or OpenCode heads; the effort slider fuses the two colours for a pair.
- The team's panel and any head popped out of it float over the chat, dock in any corner, stack when they share one, and can be dragged to size; a finished head wears its outcome on its glyph, and every head has its own progress bar with its task and time.
- Command Code joins the providers, with approvals, plan limits and remaining credits.
- Settle a thread: it drops to the bottom of the sidebar, small and grey, until you reopen it.
- A welcome tour on first launch, six pages of real captures; Help › Welcome tour and About open it again.
- A floating usage panel beside the chat, switched on in General settings, with the plan's limits and credits for the model in use or a pair's two; the usage popover's expand button opens it any time.
- The usage popover stacks the lead's provider over the heads' when a pair crosses providers.
- A Threads button brings the hidden sidebar's list up in a popover, on hover.
- A zoom slider in the chrome row scales the conversation alone.
- Continue after a usage limit: when the provider's limit is spent, the chat waits for the reset and tells the agent to carry on.
- Deleting a thread asks in a popover on its row.

### Bug fixes
- The chat no longer shows empty space after idling, a quick thread switch or a turn folding; the timeline repairs itself when it finds nothing on screen.
- The window no longer freezes after typing, a loop in the queue tab's chevron.
- The working line moves as one piece, and the traffic lights take their clicks with the sidebar hidden.
- API keys live in the Keychain alone: the copy the defaults held in plain text is gone, and old copies are cleared at launch.
- Attachment files no thread refers to any more are swept from disk a day after they were written.

### Refinements
- Long threads run at 120 fps: rows are laid out as they come into view, blocks are cached, and streamed chunks are coalesced off the main thread.
- The effort slider's knob is a Liquid Glass lens, and popover buttons are the window's native capsules.
- The model picker keeps its glass inside a popover, with one checkmark for the choice.
- Heads have a budget on every provider, a watchdog stops the ones that stall, and finished heads can clear themselves from the panel.
- The sidebar's traffic lights move to it only once it can hold them, and the rail only answers over its ticks.


## [1.1.1] - 2026-09-14

A small release on the heels of 1.1.0: the chat never opens onto empty space, the working line arrives like every other row, and every copy of the app carries its licenses. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Settings › About › Licenses shows the app's license, the third-party notices and the trademark policy, shipped inside the app.

### Bug fixes
- The chat could show nothing at all after a turn finished or a quick thread switch, until the thread was opened again; the timeline now snaps back to the conversation's end whenever it finds itself past it or off its place.
- The working line appeared with a buggy slide; it is a row of the timeline now and arrives like the message you just sent.
- With the sidebar hidden, the window buttons sat too high beside the chat's chrome; they drop to its centre line as they move over.

### Refinements
- About no longer repeats the version above the release cards.


## [1.1.0] - 2026-09-14

Droppy Code 1.1.0 updates itself from here on: Settings › About checks for new versions and installs them with Update & restart. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Droppy Code updates itself: About checks GitLab for new versions, shows what changed, and Update & restart installs it in place.
- GitHub Copilot CLI joins the providers, with its models, efforts and monthly quotas.
- Antigravity joins the providers, with its plan limits.
- DeepSeek and Meta (Muse Spark) are native API providers, keys kept in your Keychain, with DeepSeek's remaining credits in the usage popover.
- 26 tinted-glass themes, from System to Catppuccin, Claude and Codex, and a window transparency setting.
- A follow-up queue above the chat box: steer a running turn, reorder what is queued, or send a queued prompt now.
- The agent's questions take a tab on the chat box, so nothing waits unseen.
- Right-click a merge or pull request link for Merge, which lands it in a helper thread.
- A minimap rail beside long threads, large previews for photos and the files the agent read, recent downloads from the attach button, and a soft chime when a turn finishes.
- Token activity heatmap at the top of General settings, and every signed-in account's usage limits on the Providers page.

### Bug fixes
- The working line appears in place instead of sliding up from under the chat box.
- Native providers' turns no longer stop after twelve tool rounds, and DeepSeek tool-call chains survive trims, compaction and interrupts.
- The changes popover opens on the file a tapped tool row edited, even when the turn's diff does not list it.
- Tool rows no longer read "Edited Edit file".
- Every attachment photo opens its own preview, not only the latest one.
- Link paragraphs no longer come out blank, and sending no longer freezes the window.
- The scroll veil at the top of a pane draws correctly in dark mode.

### Refinements
- Long threads scroll smoothly: rows are built as they come into view, hover rests while you scroll, and expanded rows stay put.
- Finished turns show only their final answer, with the tool run folded behind a chevron.
- Your messages sit in iMessage's bubble.
- Right-clicking a link opens the app's own popover instead of the system's text menu.
- Remaining credits is a settings row, and an empty archive is one symbol in the middle of the pane.
- Spinners and links wear the theme's accent; provider logos share one optical size, with Meta and DeepSeek in their brand colours.
- Popover buttons are the window's capsules, the chat box grows smoothly to five lines, and suggestions anchor to the caret.


## [1.0.0] - 2026-09-13

The coding app by Droppy (https://getdroppy.app): a native Mac app for your coding agents. Apple silicon, macOS 26 or later. Signed and notarized.

### New features
- Codex, Claude, Cursor, OpenCode, Grok, DeepSeek, Meta, Devin and Antigravity, each in its own thread, in one native window.
- The agent's questions and approvals take a tab on the chat box, so nothing waits unseen.
- Follow-ups queue above the chat box while a turn runs and send themselves when it ends.
- A diff for every turn, behind the changes tab, with a review popover.
- The command palette: every thread, project and action one keystroke away.
- The reasoning-effort slider and the model switcher, right in the composer.
- Projects and threads in the sidebar, with an Activity view that keeps the running ones at hand.
- Plans and to-dos as the agent writes them, and a notification when a turn finishes.
- Themes for the whole window, in light and dark: Catppuccin, Dracula, Tokyo Night, Nord, Gruvbox, Solarized, GitHub, Matrix and more.
- A worktree per thread, so parallel agents never touch the same checkout.
- A terminal under every thread.
