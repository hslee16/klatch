ACP specification check for the librarian agent (2026-11-20, ticket #869)
The Agent Client Protocol pages at agentclientprotocol.com were read on 2026-11-20 and compared with `klatch acp --stdio`. The per-item comparison is in the design item for #869.

### What the spec fixes and what it leaves open
- Fixed: integer `protocolVersion` (1), `agentCapabilities` booleans default to unsupported, a `sessionId` from `session/new`, `agent_message_chunk` updates, the `stopReason` list, `session/cancel` as a notification answered by a `cancelled` stop reason on the prompt, one message per line on stdio with nothing else on stdout.
- Open: what `authenticate` answers when no method is advertised; whether a refused query should be `refusal` or text with `end_turn`; whether a missing or unknown `sessionId` is invalid params or resource not found (-32002).
- Optional: `session/load`, slash commands, plans, modes.

### Limits of this check
The check is of published text, read through a page summarizer, not of the JSON schema file itself and not of any editor. No claim is made that Zed or JetBrains accept the agent.
