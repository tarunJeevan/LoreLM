# Phase 1 Notes

## `handle_key` in `app-tui`

- There is a Tab + no modifier key combo that simply appends `\t` to the prompt. This is fine. There's another match section for Tab used to cycle through which panel is focused. There is currently no check on this match section to see if the user is currently focused on `Panel::Prompt`. The correct user flow if focus is currently on `Panel::Prompt` should be that the user presses `Esc` to switch focus from the prompt panel and then use `Tab` to cycle focus between different panels. There should be logic when handling `Tab` and `Backtab` to confirm that the current focus is NOT on `Panel::Prompt` before going through the UI focus cycling logic.
