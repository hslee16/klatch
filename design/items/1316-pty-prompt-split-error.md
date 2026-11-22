The pty test helper does not type at a half-written `Error: ` line (#1316; no schema change; 2026-11-22).
The pty helper used by the admin proof tests types a line whenever the terminal output ends in `: ` or `> `. A command that fails prints `Error: ` and then the message as separate writes, so a read can end right after `Error: `, which also ends in `: `. The helper then typed a line at it while echo was on, spent that line, and left the real next passphrase prompt unanswered, so a test that expected three wrong passphrases hung until its deadline on Linux CI.

A last line that starts with `Error:` is now never taken for a prompt (`is_prompt` in the helper, with a test). The product is unchanged.
