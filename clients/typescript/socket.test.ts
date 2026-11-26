// Characterisation tests for the socket and stdio paths of the TypeScript client (#1443). Run from this directory:
// node --experimental-strip-types --test socket.test.ts (http.test.ts imports this file, so CI runs it with that one).
//
// They run against a small in-process server on a Unix socket that speaks the wire of the klatch-server daemon (a hello,
// then one answer per request line), and against a tiny fake gateway process for stdio. They pin what the client does
// today: the request framing, how a request skips the frames that are not its answer, what `identify` changes, and what
// `close` does, so the internals can be reorganised without a change in behaviour. The real daemon is exercised by
// conformance.test.ts. Every wait has a deadline.
import assert from "node:assert/strict";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { createServer, type Server, type Socket } from "node:net";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, beforeEach, describe, it } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { ApiError, Client, ProtocolError } from "./klatch.ts";

const HELLO = { type: "hello", member: "m-1", attribution: "typed", clearance: "public" };
type Out = Record<string, unknown> | string;
type Answer = (frame: any) => Out[];

const line = (frame: Record<string, unknown>) => JSON.stringify({ v: 1, ...frame }) + "\n";
const ok = (id: string, result?: Record<string, unknown>) => ({ type: "response", id, ok: true, ...(result === undefined ? {} : { result }) });
const defaultAnswer: Answer = (f) => [ok(f.id, {})];

async function until(cond: () => boolean, ms = 5_000): Promise<void> {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error("the condition never held");
    await sleep(10);
  }
}

class FakeDaemon {
  dir = mkdtempSync(join(tmpdir(), "kc-"));
  path = join(this.dir, "d.sock");
  received: string[] = [];
  ended = false;
  conns: Socket[] = [];
  answer: Answer = defaultAnswer;
  hello: Record<string, unknown> | null = HELLO;
  private srv: Server;

  constructor() {
    this.srv = createServer((conn) => this.session(conn));
  }

  async listen() {
    await new Promise<void>((r) => this.srv.listen(this.path, r));
  }

  private session(conn: Socket) {
    this.conns.push(conn);
    conn.on("error", () => {});
    if (this.hello !== null) conn.write(line(this.hello));
    let buf = "";
    conn.setEncoding("utf8");
    conn.on("data", (chunk: string) => {
      buf += chunk;
      for (let i = buf.indexOf("\n"); i >= 0; i = buf.indexOf("\n")) {
        const text = buf.slice(0, i);
        buf = buf.slice(i + 1);
        this.received.push(text);
        this.answerLine(conn, text);
      }
    });
    conn.on("end", () => {
      this.ended = true;
      conn.end();
    });
  }

  private answerLine(conn: Socket, text: string) {
    const frame = JSON.parse(text);
    if (frame.type !== "request") return;
    for (const out of this.answer(frame)) conn.write(typeof out === "string" ? out : line(out));
  }

  frames(): any[] {
    return this.received.map((l) => JSON.parse(l));
  }

  dropClients() {
    for (const c of this.conns) c.destroy();
  }

  async stop() {
    this.dropClients();
    await new Promise<void>((r) => this.srv.close(() => r()));
    rmSync(this.dir, { recursive: true, force: true });
  }
}

describe("klatch client over the daemon's Unix socket", () => {
  let d: FakeDaemon;
  let clients: Client[];
  beforeEach(async () => {
    d = new FakeDaemon();
    await d.listen();
    clients = [];
  });
  afterEach(async () => {
    for (const c of clients) await c.close();
    await d.stop();
  });

  async function connect(identity: Record<string, string> = {}): Promise<Client> {
    const c = await Client.connectSocket(d.path, identity);
    clients.push(c);
    return c;
  }

  describe("connectSocket", () => {
    it("sends no identify without an identity and takes the hello", async () => {
      const c = await connect();
      assert.deepEqual([c.member, c.attribution, c.clearance], ["m-1", "typed", "public"]);
      assert.deepEqual(d.received, []);
    });

    it("identifies a name first, and the answer replaces member and attribution", async () => {
      d.answer = (f) => [ok(f.id, { member: "alice", attribution: "unattributed" })];
      const c = await connect({ name: "alice" });
      assert.deepEqual([c.member, c.attribution, c.clearance], ["alice", "unattributed", "public"]);
      assert.deepEqual(d.received, ['{"v":1,"type":"request","id":"c1","method":"identify","params":{"name":"alice"}}']);
    });

    it("identifies a member and token as such", async () => {
      d.answer = (f) => [ok(f.id, { member: "w-1", attribution: "verified" })];
      const c = await connect({ member: "w-1", token: "tok" });
      assert.deepEqual([c.member, c.attribution], ["w-1", "verified"]);
      assert.deepEqual(d.frames()[0].params, { member: "w-1", token: "tok" });
    });

    it("keeps the hello values when the answer has no member or attribution", async () => {
      const c = await connect({ name: "alice" });
      assert.deepEqual([c.member, c.attribution], ["m-1", "typed"]);
    });

    it("rejects a bad identity with a TypeError before any connect", async () => {
      const nobody = join(tmpdir(), "kc-no-such.sock");
      const cases: [Record<string, string>, RegExp][] = [
        [{ name: "a", member: "w" }, /not both/],
        [{ name: "a", token: "t" }, /not both/],
        [{ member: "w" }, /a member and a token/],
        [{ token: "t" }, /a member and a token/],
      ];
      for (const [identity, text] of cases) {
        await assert.rejects(Client.connectSocket(nobody, identity), (e: Error) => e instanceof TypeError && text.test(e.message));
      }
    });

    it("rejects with the socket's error for a path nobody listens on", async () => {
      await assert.rejects(Client.connectSocket(join(d.dir, "nobody.sock")), (e: NodeJS.ErrnoException) => e.code === "ENOENT");
    });

    it("rejects a refused identity with an ApiError and closes the socket", async () => {
      d.answer = (f) => [{ type: "response", id: f.id, ok: false, error: { code: "identity.refused", message: "no" } }];
      await assert.rejects(Client.connectSocket(d.path, { name: "mallory" }), (e: ApiError) => e instanceof ApiError && e.code === "identity.refused" && e.detail === "no");
      await until(() => d.ended);
    });

    it("rejects a first frame that is not hello with a ProtocolError", async () => {
      d.hello = { type: "heartbeat" };
      await assert.rejects(Client.connectSocket(d.path), ProtocolError);
    });

    it("rejects with a ProtocolError when the gateway goes before it says hello", async () => {
      d.hello = null;
      setTimeout(() => d.dropClients(), 300);
      await assert.rejects(Client.connectSocket(d.path), ProtocolError);
    });
  });

  describe("request", () => {
    it("frames compact JSON with version, type, id, method, params; ids count up", async () => {
      const c = await connect();
      assert.deepEqual(await c.request("who", { room: "lobby" }), {});
      assert.deepEqual(await c.request("join", { room: "r2" }), {});
      assert.deepEqual(d.received, [
        '{"v":1,"type":"request","id":"c1","method":"who","params":{"room":"lobby"}}',
        '{"v":1,"type":"request","id":"c2","method":"join","params":{"room":"r2"}}',
      ]);
    });

    it("sends empty params when none are given", async () => {
      const c = await connect();
      await c.request("prune");
      assert.equal(d.received[0], '{"v":1,"type":"request","id":"c1","method":"prune","params":{}}');
    });

    it("answers an empty object for a response without a result", async () => {
      d.answer = (f) => [ok(f.id)];
      assert.deepEqual(await (await connect()).request("prune"), {});
    });

    it("returns the result as is", async () => {
      d.answer = (f) => [ok(f.id, { head: 7 })];
      assert.deepEqual(await (await connect()).request("subscribe", { room: "r" }), { head: 7 });
    });

    it("throws an ApiError with code and message for an error response", async () => {
      d.answer = (f) => [{ type: "response", id: f.id, ok: false, error: { code: "refused", message: "rule x" } }];
      const c = await connect();
      await assert.rejects(c.request("post", { room: "r", body: "b" }), (e: ApiError) => {
        assert.deepEqual([e.code, e.detail, e.message, e.refusals], ["refused", "rule x", "refused: rule x", []]);
        return e instanceof ApiError;
      });
    });

    it("uses the code backend for an error response without one", async () => {
      d.answer = (f) => [{ type: "response", id: f.id, ok: false }];
      const c = await connect();
      await assert.rejects(c.request("x"), (e: ApiError) => e.code === "backend" && e.detail === "");
    });

    it("attaches refusal frames that came before the answer, and resets them per request", async () => {
      const refusal = { type: "refusal", rule: "scope", why: "w" };
      d.answer = (f) =>
        f.method === "bad" ? [refusal, { type: "response", id: f.id, ok: false, error: { code: "refused", message: "m" } }] : [ok(f.id, {})];
      const c = await connect();
      await assert.rejects(c.request("bad"), (e: ApiError) => e.refusals.map((r) => r.rule).join() === "scope");
      assert.deepEqual(c.refusals.map((r) => r.rule), ["scope"]);
      await c.request("fine");
      assert.deepEqual(c.refusals, []);
    });

    it("keeps frames that arrive meanwhile and drops the noise", async () => {
      const msg = { type: "message", room: "r", seq: 1, body: "hi" };
      d.answer = (f) => [
        "\n",
        "this is not json\n",
        { type: "heartbeat" },
        { type: "never-heard-of-it", extra: 1 },
        { type: "response", id: "someone-else", ok: true, result: { wrong: true } },
        msg,
        { type: "caught_up", room: "r", head: 1 },
        ok(f.id, { head: 1 }),
      ];
      const c = await connect();
      assert.deepEqual(await c.request("subscribe", { room: "r", from_seq: 0 }), { head: 1 });
      const got = await c.collect();
      assert.deepEqual(got.messages, [{ v: 1, ...msg }]);
      assert.deepEqual(got.caughtUp, { v: 1, type: "caught_up", room: "r", head: 1 });
      assert.deepEqual((await c.collect()).messages, []);
    });

    it("rejects with a ProtocolError when the gateway closes without answering", async () => {
      d.answer = () => [];
      const c = await connect();
      setTimeout(() => d.dropClients(), 300);
      await assert.rejects(c.request("who", { room: "r" }), (e: Error) => e instanceof ProtocolError && /closed the session or did not answer/.test(e.message));
    });

    it("ignores a frame of another protocol version and carries on", async () => {
      d.answer = (f) => ['{"v":2,"type":"message"}\n', ok(f.id, { fine: true })];
      assert.deepEqual(await (await connect()).request("who", { room: "r" }), { fine: true });
    });

    it("rejects with a ProtocolError after close, and sends nothing", async () => {
      const c = await connect();
      await c.close();
      await assert.rejects(c.request("who", { room: "r" }), (e: Error) => e instanceof ProtocolError && e.message === "the session is closed");
      assert.deepEqual(d.received, []);
    });
  });

  describe("methods", () => {
    async function wire(call: (c: Client) => Promise<unknown>, result: Record<string, unknown> = {}): Promise<any> {
      d.answer = (f) => [ok(f.id, result)];
      const c = await connect();
      await call(c);
      return d.frames().at(-1);
    }

    it("post keeps only the given optionals", async () => {
      let f = await wire((c) => c.post("lobby", "b"));
      assert.deepEqual(f.params, { room: "lobby", body: "b" });
      f = await wire((c) => c.post("lobby", "seeking: x", { label: "L", scope: "s", refs: [{ kind: "pr", value: "1" }] }));
      assert.deepEqual(f.params, { room: "lobby", body: "seeking: x", label: "L", scope: "s", refs: [{ kind: "pr", value: "1" }] });
      f = await wire((c) => c.post("lobby", "b", { label: undefined, kind: "", refs: [] }));
      assert.deepEqual(f.params, { room: "lobby", body: "b", kind: "", refs: [] });
    });

    it("dm, history, claim, release and reviewVerdict leave out what is undefined", async () => {
      assert.deepEqual((await wire((c) => c.dm("bob", "hi"))).params, { to: "bob", body: "hi" });
      assert.deepEqual((await wire((c) => c.dm("bob", "hi", "L"))).params, { to: "bob", body: "hi", label: "L" });
      assert.deepEqual((await wire((c) => c.history("r"), { messages: [] })).params, { room: "r" });
      assert.deepEqual((await wire((c) => c.history("r", { last: 5, since: 2 }), { messages: [] })).params, { room: "r", last: 5, since: 2 });
      assert.deepEqual((await wire((c) => c.claim("r", "a/b"))).params, { room: "r", path: "a/b" });
      assert.deepEqual((await wire((c) => c.claim("r", "a/b", { ttl: 0, reason: "x", shared: false }))).params, { room: "r", path: "a/b", ttl: 0, reason: "x", shared: false });
      assert.deepEqual((await wire((c) => c.release("r", "p"))).params, { room: "r", path: "p" });
      assert.deepEqual((await wire((c) => c.release("r", "p", "n"))).params, { room: "r", path: "p", note: "n" });
      assert.deepEqual((await wire((c) => c.reviewVerdict(3, "deny", { reason: "r" }))).params, { id: 3, verdict: "deny", reason: "r" });
    });

    it("history defaults more, read sends peek, roleAct puts act last", async () => {
      d.answer = (f) => [ok(f.id, { messages: [1] })];
      const c = await connect();
      assert.deepEqual(await c.history("r"), { messages: [1], more: 0 });
      assert.deepEqual(await c.read("r"), [1]);
      assert.deepEqual(d.frames().at(-1).params, { room: "r", peek: false });
      await c.roleAct("close", { room: "r" });
      assert.equal(d.received.at(-1), '{"v":1,"type":"request","id":"c3","method":"role_act","params":{"room":"r","act":"close"}}');
    });

    it("read sends maxMessages and maxBytes as max_messages and max_bytes only when given", async () => {
      assert.deepEqual((await wire((c) => c.read("r"), { messages: [] })).params, { room: "r", peek: false });
      const f = await wire((c) => c.read("r", true, { maxMessages: 3, maxBytes: 2000 }), { messages: [] });
      assert.deepEqual(f.params, { room: "r", peek: true, max_messages: 3, max_bytes: 2000 });
      assert.deepEqual((await wire((c) => c.read("r", false, { maxMessages: 0 }), { messages: [] })).params, { room: "r", peek: false, max_messages: 0 });
      assert.deepEqual((await wire((c) => c.read("r", false, { maxBytes: undefined }), { messages: [] })).params, { room: "r", peek: false });
    });

    it("readBounded answers the messages, omitted and byImportance", async () => {
      d.answer = (f) => [ok(f.id, { messages: [1], omitted: 4, by_importance: true })];
      const c = await connect();
      assert.deepEqual(await c.read("r", false, { maxMessages: 1 }), [1]);
      assert.deepEqual(await c.readBounded("r", false, { maxMessages: 1 }), { messages: [1], omitted: 4, byImportance: true });
      assert.deepEqual(d.frames().at(-1).params, { room: "r", peek: false, max_messages: 1 });
      d.answer = (f) => [ok(f.id, { messages: [1] })];
      assert.deepEqual(await c.readBounded("r", true), { messages: [1], omitted: null, byImportance: false });
      assert.deepEqual(d.frames().at(-1).params, { room: "r", peek: true });
    });

    it("a bound the server refuses is an ApiError and was sent as given", async () => {
      d.answer = (f) => [{ type: "response", id: f.id, ok: false, error: { code: "bad_params", message: "max_messages" } }];
      const c = await connect();
      await assert.rejects(c.read("r", false, { maxMessages: -1 }), (e: Error) => e instanceof ApiError && e.code === "bad_params");
      assert.equal(d.frames().at(-1).params.max_messages, -1);
    });

    it("ack is a frame, not a request, and waits for no answer", async () => {
      const c = await connect();
      c.ack("r", 4);
      await c.request("who", { room: "r" });
      assert.equal(d.received[0], '{"v":1,"type":"ack","room":"r","seq":4}');
    });
  });

  describe("the stream", () => {
    const msg = (seq: number) => ({ type: "message", room: "r", seq, body: String(seq) });

    it("subscribe answers the head and collect waits for caught_up", async () => {
      d.answer = (f) =>
        f.method === "subscribe" ? [ok(f.id, { head: 2 }), msg(1), msg(2), { type: "caught_up", room: "r", head: 2 }] : [ok(f.id, {})];
      const c = await connect();
      assert.equal(await c.subscribe("r"), 2);
      const got = await c.collect(true, 5_000);
      assert.deepEqual(got.messages.map((m) => m.seq), [1, 2]);
      assert.equal((got.caughtUp as any).head, 2);
    });

    it("collect without waiting returns what is there, and null for caughtUp", async () => {
      assert.deepEqual(await (await connect()).collect(), { messages: [], caughtUp: null });
    });

    it("collect until caught up gives up at the deadline", async () => {
      assert.deepEqual(await (await connect()).collect(true, 200), { messages: [], caughtUp: null });
    });

    it("nextMessage pops in order and is null on timeout", async () => {
      d.answer = (f) => [ok(f.id, { head: 2 }), msg(1), msg(2)];
      const c = await connect();
      await c.subscribe("r");
      assert.equal((await c.nextMessage(5_000))?.seq, 1);
      assert.equal((await c.nextMessage(5_000))?.seq, 2);
      assert.equal(await c.nextMessage(200), null);
    });

    it("nextMessage is null when the gateway has gone", async () => {
      const c = await connect();
      d.dropClients();
      assert.equal(await c.nextMessage(5_000), null);
    });
  });

  describe("close", () => {
    it("ends the write side so the daemon sees the end, and answers 0", async () => {
      const c = await Client.connectSocket(d.path);
      assert.equal(await c.close(), 0);
      await until(() => d.ended);
    });

    it("answers 0 again when called twice", async () => {
      const c = await Client.connectSocket(d.path);
      assert.equal(await c.close(), 0);
      assert.equal(await c.close(), 0);
    });

    it("answers 0 after the daemon dropped the session", async () => {
      const c = await Client.connectSocket(d.path);
      d.dropClients();
      assert.equal(await c.close(), 0);
    });
  });
});

const GATEWAY = `
const out = (f) => process.stdout.write(JSON.stringify({ v: 1, ...f }) + "\\n");
out({ type: "hello", member: "p-1", attribution: "session", clearance: "public" });
let buf = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buf += chunk;
  for (let i = buf.indexOf("\\n"); i >= 0; i = buf.indexOf("\\n")) {
    const f = JSON.parse(buf.slice(0, i));
    buf = buf.slice(i + 1);
    out({ type: "response", id: f.id, ok: true, result: { method: f.method, args: process.argv.slice(2) } });
  }
});
process.stdin.on("end", () => process.exit(3));
`;

describe("klatch client over a gateway process (stdio)", () => {
  let dir: string;
  let script: string;
  beforeEach(() => {
    dir = mkdtempSync(join(tmpdir(), "kc-gw-"));
    script = join(dir, "gateway.cjs");
    writeFileSync(script, GATEWAY);
  });
  afterEach(() => rmSync(dir, { recursive: true, force: true }));

  it("reads the process hello, answers a request, and close returns the exit code", async () => {
    const c = await Client.connect(process.execPath, {}, [script, "a", "b"]);
    assert.deepEqual([c.member, c.attribution, c.clearance], ["p-1", "session", "public"]);
    assert.deepEqual(await c.request("who", { room: "r" }), { method: "who", args: ["a", "b"] });
    assert.equal(await c.close(), 3);
  });

  it("answers the same exit code when close is called twice", async () => {
    const c = await Client.connect(process.execPath, {}, [script]);
    assert.equal(await c.close(), 3);
    assert.equal(await c.close(), 3);
  });

  it("rejects the connect with a ProtocolError when the process exits at once", async () => {
    writeFileSync(script, "process.exit(0);\n");
    await assert.rejects(Client.connect(process.execPath, {}, [script]), ProtocolError);
  });
});
