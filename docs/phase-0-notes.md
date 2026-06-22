# Notes on phase 0

## Notes on TUI loop in app_tui

### handle_key() function

- Current key combination for entering a newline without entering is Alt+Enter. Changing this to Shift+Enter would be more ergonomic and in-line with other chatbots and LLM apps.
- Current key combination for leaving the prompt window and set focus to the wider app is the Tab key. Changing this to Esc is more ergonomic and allows the Tab key to be used for adding a '\t' to the prompt if needed.
- Scrolling behavior with Page Up and Page Down exists but line navigation with Arrow keys is not explicitly implemented (though it might be supported out of the box in some way) so it requires checking.
