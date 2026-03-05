# Troubleshooting

## Terminal Looks Corrupted After Exit

This can happen if the process is interrupted unexpectedly. Reset the terminal:

```bash
reset
```

## Key Presses Do Not Behave as Expected

- Confirm you are in the expected mode in the status bar.
- Use `Esc` to return to normal mode, then retry.

## File Did Not Save

- Verify you used `:w` or `:wq`.
- Check write permissions for the target path.
- Confirm the status/message line for save errors.

## Configuration Warnings at Startup

- Ordex prints configuration warnings to stderr and still starts when recovery is possible.
- Check for unknown keys, invalid values, or missing include files in your config.
- If a keymap does not apply, verify the action/key syntax and mode section.
