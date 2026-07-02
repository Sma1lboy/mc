---
name: ladle-ui
description: >-
  Debug and iterate on the desktop UI (React components) in isolation with Ladle —
  no need to launch the whole Tauri app. Use when tweaking the look of any
  component (chat message parts, ask_user chips, cards, dialogs, primitives),
  reviewing a component's states, or verifying a visual change via screenshot.
  Triggers: "调 UI / tweak this component / show me the card / preview the chat
  parts / iterate on styling / screenshot this story".
---

# Ladle UI sandbox

Ladle (`@ladle/react`) renders each UI component in isolation from `*.stories.tsx`
files, with the app's real Tailwind v4 + theme tokens. This is the fast loop for
UI work: **edit component → HMR → screenshot → repeat**, without building/running
the Tauri app or hand-driving screenshots.

## Run it

```bash
npm run ladle -w desktop        # dev server → http://localhost:61000
# or from desktop/:  npm run ladle
npm run ladle:build -w desktop  # static build (also what CI-style verify uses)
```

Stories live in `desktop/src/agent/*.stories.tsx` (chat UI) and
`desktop/src/components/*.stories.tsx` (primitives/cards/dialogs). The `.ladle/`
folder holds the config + a global Provider that loads `fonts.css` / `tokens.css`
/ `tailwind.css` and wraps every story in the dark app shell — so stories match
the real app with zero per-story setup.

## The debug loop (edit → screenshot → repeat)

1. Start `ladle serve` (leave it running; it HMRs).
2. Open a story's **direct URL** and screenshot it (use `/browse` — the project's
   browser per the workspace CLAUDE.md):
   `http://localhost:61000/?story=<story-id>`
   Story ids are `<title>--<storyName>` kebab-cased (CJK kept), e.g.
   `agent--chat--完整对话流-·-messages-list-请求-flow-`. List them all from
   `http://localhost:61000/meta.json` (`.stories` keys).
3. Edit the component source under `desktop/src/…` — Ladle HMRs instantly.
4. Re-screenshot the same URL to verify. Iterate.

This replaces the old screenshot gallery (`desktop/src/gallery/runner.ts`, which
drives the *real* Tauri window for release shots) for day-to-day UI tweaking.

## Writing a new story

- One `export const X: Story = () => <Component … />` per state; set
  `X.storyName` for a readable label; group with `export default { title: "…" }`.
- Feed **mock props**. For chat UI, construct mock `UIMessage` / `ToolUIPart`
  objects (the state machine is `input-streaming` → `input-available` →
  `output-available`); see `desktop/src/agent/agentChat.stories.tsx` for the
  `askPart` / `toolPart` / `flowMsg` helpers and the full-conversation-flow story.
- Cover a component's **states** as separate stories (loading / empty / error /
  selected / answered), not just the happy path — that's the point of the sandbox.

## Constraints

- Components that read the zustand store or call Tauri `invoke()` have **no
  backend** in Ladle. Pass mock props; seed the store in the story if it reads it
  directly; stub callbacks (e.g. `submitAskUserAnswer` no-ops without Tauri). If a
  component genuinely can't render without Tauri, skip it with a one-line note —
  don't fake internals that misrepresent it.
- Story files are dev-only: literal strings are fine (the i18n check only scans
  `t()` calls, so `node scripts/check-i18n.mjs` stays green).

## Before committing

From `desktop/`: `npx tsc --noEmit` and `npx ladle build` must pass;
`node scripts/check-i18n.mjs` stays green. Commit stories alongside the component
they cover (Conventional Commits, no AI attribution).
