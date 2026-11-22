The admin passphrase prompt restores the terminal when a signal ends the process (#1278; no schema change; 2026-11-22).
While the passphrase is read, echo and signal keys are off. A SIGTERM, SIGINT, SIGHUP or SIGQUIT from outside now runs a handler that puts the saved terminal settings back and then re-raises the signal with its default action, so the process still ends the way the sender meant and the terminal is not left silent. The handler uses only async-signal-safe calls, and the old signal actions are restored when the prompt ends.

SIGKILL cannot be caught, so a kill -9 during the prompt still leaves echo off (`stty sane` fixes it). There is no reliable reset on the next start, since a later process cannot know what the terminal was before.
