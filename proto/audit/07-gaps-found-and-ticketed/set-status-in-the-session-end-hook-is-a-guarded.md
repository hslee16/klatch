| `set_status` in the session-end hook is a guarded call, so an idle-expired credential cannot mark the member `done` at session end (claims are still freed) | #1027 |
