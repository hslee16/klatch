The unlock window also counts the wall clock (#1277; no schema change; 2026-11-22).
The unlock window of the admin proof (#1233) was measured only on the monotonic clock (`Instant`), which on macOS does not advance while the machine sleeps, so a window could outlive its length across a sleep. It now records both the monotonic and the wall clock when it opens and ends when either has passed the window length.

A wall clock set back cannot hold a window open, because the monotonic clock still ends it; a wall clock set forward ends it early, which only asks for the passphrase again.
