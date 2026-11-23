# Anti-slop guidance

Bottega carries a short, built-in block of guidance that asks the model for the shortest complete answer: leading with the result, skipping filler and hype, keeping comments only where they explain a non-obvious reason, and making focused edits instead of unrelated cleanup. The same block asks for product-fitting UI structure and for evidence to be reused rather than re-read.

## Off by default, and what the setting does

The guidance is off by default. It is exposed in **Settings › General › Conversation** as **Concise AI replies**. Droppy Code, which Bottega is built from, shipped it on; Bottega turned it off because it steers every session, Crew hands included, toward brevity and away from anything beyond the task, which works against a workflow that asks for thorough checks and for every instance of a root cause to be fixed. The switch stays for anyone who wants the guidance.

While the switch is off, Bottega sends the matching "off" block instead, which tells the model to disregard earlier app-provided guidance and to keep following user requests and project requirements. It goes out the same way the guidance does, so a resumed conversation that was started with the guidance on is told to set it aside.

The setting takes effect on the next message, which starts the session again with the new block and resumes the same conversation. A message already in flight is not changed, and toggling the switch does not rewrite the current conversation or thread history.

Claude, Codex, Copilot and the API-key providers carry the block in their system prompt (Codex in its developer instructions) from the moment their session starts or resumes. The other providers receive it with the first ordinary message of a session, and again after the session is restarted or explicitly compacted; slash commands defer delivery until that ordinary message. Automatic history changes inside those providers are not observable by the app.

Nothing is promised about token savings. Shorter replies and fewer redundant reads usually use fewer tokens, but the model, the task and the provider's own behavior all vary, so no fixed saving is guaranteed and no such claim is made in the UI.

## All providers, including Crew hands

The guidance is attached to the provider request, so it reaches every provider Bottega runs, not just one. Crew hands follow the same switch as their lead, on or off:

- Native hands on Claude and Copilot carry the block in the agent definitions Bottega gives the CLI: the worker, the scout and each profile's worker variant.
- A hand Bottega hosts on another provider is a thread of its own, and receives the block the way any thread does.
- Native hands on Codex are defined by Codex itself, and Bottega gives them no definition of its own; there the block sits in the lead's developer instructions, and it asks that the guidance apply to delegated work too.

## A compact authored adaptation, not upstream tooling

The guidance in `Bottega/Core/Models/AntiSlopPolicy.swift` is a compact adaptation authored for Droppy Code and carried into Bottega with only its label renamed. It is not an installation of the upstream anti-slop project, and it does not run any of its tooling.

Upstream inspiration: [miqdadbadjuber/anti-slop](https://github.com/miqdadbadjuber/anti-slop) at revision [`743735248fbaefd76bb56619615687dfa8b3bc1e`](https://github.com/miqdadbadjuber/anti-slop/tree/743735248fbaefd76bb56619615687dfa8b3bc1e), MIT licensed, copyright 2026 Miqdad Badjuber. The upstream [LICENSE](https://github.com/miqdadbadjuber/anti-slop/blob/743735248fbaefd76bb56619615687dfa8b3bc1e/LICENSE) applies to that work.

What is intentionally left out of Bottega:

- The upstream wizard and its interactive setup.
- Mandatory audit reports.
- External executables or scripts the upstream project may invoke.

Bottega carries only the guidance text, as a value it can hand to a provider. It installs nothing, runs no audit, and requires no external binary.
