
The client frame `ack` calls `ack` (`Ack`, GUARDED `ack`). An unknown method answers `unknown_method` and reaches no
gateway call. The wire protocol has no floor act and no `admin` method of its own: `role_act` is the only way in and
it answers `unknown_act` for anything outside the admin agent's list. The floors have a read-only wire form only
(`label_order`, `scrub_check`, decision on #740): the scrub add, remove and list and the label chain and cap writes stay
terminal-only and answer `unknown_method` on the wire (`the_scrub_rules_and_the_label_order_writes_stay_off_the_wire`).

