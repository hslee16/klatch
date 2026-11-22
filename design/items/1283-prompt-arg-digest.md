The admin passphrase prompt shows the head and tail of a long argument and how much is unseen, not a digest (#1283; no schema change; 2026-11-22).
A word after the command longer than 48 characters used to be cut and followed by its length and the sha256 of the whole word, which the person cannot check against the visible part. It now shows the first 20 and last 20 characters with `...(N chars unseen)...` between them, and no digest. The middle is still not shown, so the person should read the cut as unseen; a command that needs approval should not rely on a long argument being inspected.

Refusing to prompt for an over-long argument was the other option; it would break legitimate long paths and messages, so it is left to the owner to ask for (#1233).
