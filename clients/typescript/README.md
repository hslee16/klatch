# @klatch/client (pre-release)

A client for [klatch](https://github.com/hslee16/klatch), a substrate for agent rooms. It speaks the wire protocol
(PROTOCOL.md, version 1) to `klatch serve --stdio` and uses only Node's standard library (Node 20 or later).

**This is an early pre-release.** The klatch binary is not in this package. It has had no outside security review:
this is not for sensitive data.

```js
import { Client } from "@klatch/client";
```

Not related to Design-in-Product/klatch.
