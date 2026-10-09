# Agent Workspace Design QA

Date: 2026-10-01

Final result: **passed** for the scoped browser UI and session-draft checks.
Native Tauri window interaction remains unverified in this pass.

## References and Scope

- Model menu: `E:/CodexCaches/tmp/codex-clipboard-5a460f75-8ddd-466e-a68e-b0662bc58e47.png`.
- Header/sidebar placement: `E:/CodexCaches/tmp/codex-clipboard-fa177230-d07e-4cce-adf5-369a106728d0.png`.
- DSH session listing contract: `E:/deepseek-harness/packages/session/session-persistence/tests/contract.ts`; sessions without events are excluded from history.
- DSH composer draft reference: `E:/deepseek-harness/packages/client/ui-conversation/src/client/skeleton/ConversationSession.tsx`.
- Reference and implementation images were opened together in the same comparison input and visually reviewed. This is a targeted adaptation of menu/toolbar behavior, not a full-window clone.

## Verification Surface

- URL: `http://localhost:1420/tests/agent-trace-browser.html`.
- Browser viewport: 1280 x 720; island workspace: 780 x 480, matching the existing xl desktop surface.
- The harness uses production Select, composer, sidebar, conversation hook and event reducer, with explicitly mocked desktop commands and model streaming.
- No real model requests, computer operations or music data access were performed for these UI checks.
- Screenshots: `tests/artifacts/workspace/model-menu.png`, `collapsed.png`, `workspace.png`.
- The sample Select outside the island exists only in the harness to verify downward placement; it is not part of the application UI.

## Visual Fidelity

| Surface | Result | Evidence |
| --- | --- | --- |
| Model menu | Passed | Neutral gray surface; one-line model names; selected model uses a right-hand check without a selected-row background. |
| Model trigger | Passed | Borderless and background-free text beside the send control; existing compact typography retained. |
| Header controls | Passed | Fold icon beside the outer AI Agent title; new-conversation icon immediately left of the current conversation title. |
| Sidebar | Passed | Expanded width 184 px; collapses to zero with opacity/width animation; no leftover rail or inaccessible off-screen buttons. |
| Spacing and bounds | Passed | No clipped model labels, overlapping controls or unnecessary scrollbar in the two-model menu. Menu remains inside the island's clipped/clickable bounds. |

The app retains its current typography, compact dimensions and standard library icons. The reference's model count and model names are not copied; items come from the user's enabled model configuration. Motion uses the existing Framer Motion dependency and respects reduced-motion preferences.

## Behavior Checks Executed

- Model menu opens upward near the bottom of the island; shared Select opens downward with enough space.
- Changing model moves the selected check; ArrowUp initially focuses the last enabled item, and Escape dismisses the menu.
- Fold control remains available at the outer header after collapsing, while the new-conversation icon remains in the conversation header.
- Session A and session B retain independent unsent text. An inserted computer-plugin mention survives switching and a page reload as rich editor content.
- The unsent new-conversation draft restores after visiting history and clicking New. Repeated New clicks leave the history count unchanged.
- First send creates exactly one real fixture session, derives its title from the first user message and clears only its submitted draft.
- After that send, a fresh New view is empty; existing A/B drafts remain untouched.
- Reload restores the active conversation and its draft through local browser storage. Native close/reopen uses the same storage path but was not operated directly.
- Earlier harness hot-reload createRoot warnings were corrected with root disposal. No fresh interaction/runtime exception was observed after the final reload.

## Engineering Checks

- `pnpm typecheck`: passed.
- `pnpm test:agent`: 10 passed, including popup bounds, independent rich drafts, draft promotion and storage rehydration.
- `pnpm build`: passed; existing large-chunk warning remains.
- `cargo check`: passed; existing unused-code warnings remain.
- `cargo test`: 38 passed across unit/integration suites; 2 ignored real-provider probes.
- `git diff --check`: passed.

## Data Rules

New is a reusable local draft workspace until first send. Empty stored sessions are hidden from listing, not physically deleted. Existing conversation messages are preserved. Drafts, including plugin mentions, are stored locally by conversation identity and are not sent to any provider until the user submits them. The active workspace identity is persisted; sidebar fold state and model selection persistence are outside this change.
