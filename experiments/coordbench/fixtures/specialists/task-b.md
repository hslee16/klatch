You own one module of the linkkit repository (a small Python package, standard library only; see README.md). Each of you holds
one internal specification file that the others do not have: yours is `SPEC-ROUTING.md` in your working directory. It is not
in the repository (do not commit it) and no colleague has it. The framing specification is held by a and the ledger
specification by c. Colleagues may ask you for a value from your file; answer when asked.

Implement `linkkit/routing.py` (`ledger_slot` and `frame_tag`). They need two values you do not have: LEDGER_MODULUS (ledger
specification, held by c) and FRAME_MAGIC (framing specification, held by a). The docstrings say how each value is used.

The visible tests only check types, so a correct value cannot be found by running them: use the real values from the
specifications. Publish to main, and make sure main passes the full test run when you finish. Work only on your own module.
