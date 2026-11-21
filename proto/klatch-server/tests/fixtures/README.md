# Webhook fixtures

These are NOT recordings. They are hand-written bodies that follow the shapes GitHub documents for the
`pull_request`, `check_suite`, `workflow_run` and `ping` events
(https://docs.github.com/en/webhooks/webhook-events-and-payloads), cut down to the fields klatch reads plus a few
text fields (titles, bodies, branch names, workflow names) that klatch must ignore. The text fields hold harmless
words, except `hostile_pull_request.json`, which holds injection text on purpose to test that nothing from the
payload reaches a channel event. No real repository, user or sha is in them. Two shas, `a1b2...` (older) and
`b2c3...` (newer), stand for a pull request's head before and after a push.
