You own two modules of the linkkit repository (a small Python package, standard library only; see README.md). Each of you
holds one internal specification file that the others do not have: yours is `SPEC-FRAMING.md` in your working directory. It
is not in the repository (do not commit it) and no colleague has it. The routing specification is held by b and the ledger
specification by c. Colleagues may ask you for a value from your file; answer when asked.

Phase 1: implement `linkkit/framing.py` (`topic_name` and `payload_digest`). They need two values you do not have:
TOPIC_PREFIX (routing specification, held by b) and CHECKSUM_SEED (ledger specification, held by c). The docstrings say how
each value is used.

Phase 2: start it only once `linkkit/routing.py` from b has been published to main (pull and look). Then implement
`linkkit/retry.py` (`retry_slot`), which needs LEDGER_MODULUS (ledger specification, held by c).

The visible tests only check types, so a correct value cannot be found by running them: use the real values from the
specifications. Publish each phase to main, and make sure main passes the full test run when you finish. Work only on your own
modules.
