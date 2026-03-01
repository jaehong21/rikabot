# 017 — Slash Command Autocomplete in Web UI Input

## Context

The web chat composer supports slash commands such as `/new`, `/rename`, `/clear`, `/delete`, `/stop`, and `/tools`, but command discovery and argument hints are currently limited to manual typing.

This PRD captures the work to add popup-based autocomplete for slash commands and their arguments while typing in the message input.

## Objective

1. Provide fast, in-input autocomplete for slash command names and supported arguments.
2. Render suggestions as a `shadcn` popover anchored to the composer input.
3. Keep typing uninterrupted while suggestions are open.
4. Preserve existing slash command parsing and execution behavior.

## Scope

### In scope

1. Autocomplete popup for command names when the draft starts with `/`.
2. Autocomplete popup for `/tools` sub-commands.
3. Keyboard navigation and selection behavior:
   - `↑` / `↓` navigate suggestions
   - `Tab` accepts highlighted suggestion
   - `Enter` accepts highlighted suggestion (if different), otherwise sends message
4. Click-to-accept suggestions in popup.
5. Maintain focus in the input while popup is open.
6. Remove focus/selection ring styling from suggestions if it causes unwanted blue decoration.
7. Remove slash-command command entries from the command palette so slash command execution remains available only through chat input autocomplete.

### Out of scope

1. New slash command definitions.
2. Backend protocol changes.
3. Command palette autocomplete changes.
4. Any changes to core command execution semantics.

## UX Requirements

1. When input begins with `/`, show popup suggestions near the cursor/input area.
2. Popup appears above the typing area.
3. Suggestion rows show:
   - Completion text (e.g., `/tools`, `/tools collapse`)
   - Small hint text explaining action.
4. If input changes so no suggestions apply, popup closes.
5. If input changes to plain text, popup is not shown.

## Implementation Notes

1. Add/populate command metadata in web chat route component (or shared helper):
   - Base commands: `help`, `new`, `rename`, `clear`, `delete`, `stop`, `tools`.
   - For `/tools`: subcommands `collapse`, `expand`, `hide`, `show`.
2. Add suggestion resolver from current draft + thread list (for rename name suggestions if needed).
3. Replace existing inline suggestion list with:
   - `Popover`, `PopoverAnchor`, and `PopoverContent` shadcn components.
4. Use popover open/focus handlers to keep cursor in textarea:
   - prevent auto-focus steal on open
   - return focus on close.
5. Apply non-destructive suggestion rendering style (no blue focus ring).

## Acceptance Criteria

1. Typing `/` after empty draft shows command-name suggestions.
2. Typing `/tools ` shows tool visibility subcommand suggestions.
3. Pressing `Tab` or `Enter` accepts suggestion without leaving the textarea.
4. Popup appears above the composer, not inline below the input.
5. No visible blue focus ring on selected suggestion item.
6. Existing command behaviors (`/help`, `/rename`, `/clear`, `/delete`, `/stop`, `/tools`) still execute correctly.
7. Command palette no longer includes duplicate slash-command action items (for example new/rename/clear/delete/stop/tools controls).
