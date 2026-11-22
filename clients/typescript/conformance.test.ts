// The conformance scenario for a klatch client (#119): the same steps as the Python and Rust clients, against the
// real gateway. Run from this directory:
//   KLATCH_BIN=/path/to/klatch node --experimental-strip-types --test conformance.test.ts
import assert from "node:assert/strict";
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { existsSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, afterEach, describe, it } from "node:test";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { ApiError, Client, type Message, decodeServerFrame, PROTOCOL_VERSION, ProtocolError } from "./klatch.ts";

const here = dirname(fileURLToPath(import.meta.url));
const BIN = process.env.KLATCH_BIN || resolve(here, "../../proto/target/debug/klatch");
const FIXTURES = resolve(here, "../../proto/klatch/fixtures/protocol");
const SERVER_BIN = process.env.KLATCH_SERVER_BIN || resolve(here, "../../proto/target/debug/klatch-server");

class Store {
  attribution = "unattributed"; // what the gateway says of a member of this store in the hello
  home = mkdtempSync(join(tmpdir(), "klatch-client-ts-"));
  clients: Client[] = [];
  async client(name: string): Promise<Client> {
    const env: NodeJS.ProcessEnv = { ...process.env, KLATCH_HOME: this.home, KLATCH_KEY_DIR: join(this.home, "keys"), KLATCH_NAME: name };
    delete env.KLATCH_IDENTITY_FILE;
    delete env.KLATCH_DISCOVER;
    const c = await Client.connect(BIN, env);
    this.clients.push(c);
    return c;
  }
  async close() {
    for (const c of this.clients) await c.close();
    rmSync(this.home, { recursive: true, force: true });
  }
}

/** The same throwaway store with a `klatch-server daemon` over a short socket path, and clients that connect to it over
 * the socket (#1095) by a typed name. */
class DaemonStore extends Store {
  sockdir = mkdtempSync(join("/tmp", "kc-")); // a socket path is limited to about 100 bytes
  sock = join(this.sockdir, "d.sock");
  private daemon: ChildProcess | null = null;
  async start() {
    if (this.daemon) return;
    const log = join(this.sockdir, "err.log");
    const env = { PATH: "/usr/bin:/bin", HOME: this.home, KLATCH_HOME: this.home, KLATCH_KEY_DIR: join(this.home, "keys") };
    this.daemon = spawn(SERVER_BIN, ["daemon", "--socket", this.sock], { env, stdio: ["ignore", "ignore", openSync(log, "w")] });
    for (let i = 0; i < 1500; i++) {
      if (readFileSync(log, "utf8").includes("daemon listening on")) return;
      await sleep(20);
    }
    throw new Error("the daemon never listened");
  }
  override async client(name: string): Promise<Client> {
    await this.start();
    const c = await Client.connectSocket(this.sock, { name });
    this.clients.push(c);
    return c;
  }
  stopDaemon() {
    this.daemon?.kill("SIGKILL");
  }
  override async close() {
    for (const c of this.clients) await c.close();
    this.stopDaemon();
    rmSync(this.sockdir, { recursive: true, force: true });
    rmSync(this.home, { recursive: true, force: true });
  }
}

const ISSUE_BIN = process.env.KLATCH_ISSUE_BIN || resolve(here, "../../proto/target/debug/examples/issue_test_credential");

/** The same throwaway store with a real `klatch-server http` on a port the system chooses, and clients that connect to
 * it over HTTP and server-sent events (#1114). Every connection needs a worker credential, so one is issued per member
 * with the test-only helper (`cargo build -p klatch-server --example issue_test_credential`), which refuses any store
 * that is not a throwaway one under a temp directory. The member is `verified`, proven by its credential. */
class HttpStore extends Store {
  override attribution = "verified";
  private server: ChildProcess | null = null;
  private port = 0;
  private tokens = new Map<string, string>();
  private env() {
    return { PATH: "/usr/bin:/bin", HOME: this.home, KLATCH_HOME: this.home, KLATCH_KEY_DIR: join(this.home, "keys") };
  }
  async start() {
    if (this.server) return;
    const log = join(this.home, "err.log");
    this.server = spawn(SERVER_BIN, ["http", "--listen", "127.0.0.1:0"], { env: this.env(), stdio: ["ignore", "ignore", openSync(log, "w")] });
    let text = "";
    for (let i = 0; i < 3000; i++) {
      text = readFileSync(log, "utf8");
      // The line counts only once it is complete (its newline is there), never half an address.
      const m = /http listening on 127\.0\.0\.1:(\d+) [^\n]*\n/.exec(text);
      if (m) {
        this.port = Number(m[1]);
        return;
      }
      await sleep(20);
    }
    throw new Error(`the server never listened; its stderr was:\n${text}`);
  }
  private token(name: string): string {
    let t = this.tokens.get(name);
    if (t === undefined) {
      const out = spawnSync(ISSUE_BIN, [name], { env: this.env(), encoding: "utf8", timeout: 60000 });
      if (out.status !== 0) throw new Error("the credential helper failed: " + out.stderr.trim());
      t = out.stdout.trim();
      this.tokens.set(name, t);
    }
    return t;
  }
  override async client(name: string): Promise<Client> {
    await this.start();
    const c = await Client.connectHttp(`http://127.0.0.1:${this.port}`, { member: name, token: this.token(name), connectTimeoutMs: 10000 });
    this.clients.push(c);
    return c;
  }
  override async close() {
    for (const c of this.clients) await c.close();
    this.server?.kill("SIGKILL");
    rmSync(this.home, { recursive: true, force: true });
  }
}

const daemonBuilt = existsSync(SERVER_BIN);
const httpBuilt = daemonBuilt && existsSync(ISSUE_BIN);

// The scenario runs over `klatch serve --stdio`, then unchanged over the daemon's socket: the same events and
// sequence numbers are required of every transport (and over HTTP each member has a credential issued in the store).
for (const [title, newStore, skip] of [
  ["klatch client conformance", () => new Store(), false],
  ["klatch client conformance over the daemon socket", () => new DaemonStore(), !daemonBuilt],
  ["klatch client conformance over HTTP", () => new HttpStore(), !httpBuilt],
] as [string, () => Store, boolean][])
describe(title, { skip }, () => {
  let store = newStore();
  afterEach(async () => {
    await store.close();
    store = newStore();
  });
  after(async () => store.close());

  it("speaks protocol version one", () => assert.equal(PROTOCOL_VERSION, 1));

  it("hello says who the gateway believes you are", async () => {
    const a = await store.client("alice");
    assert.deepEqual([a.member, a.attribution, a.clearance], ["alice", store.attribution, "public"]);
  });

  it("understands every server fixture line and ignores unknown things", () => {
    const lines = readFileSync(join(FIXTURES, "server_basic.ndjson"), "utf8").split("\n").filter((l) => l.trim());
    assert.ok(lines.length > 5);
    for (const line of lines) {
      const f = decodeServerFrame(line)!;
      assert.ok(["hello", "message", "caught_up", "heartbeat", "refusal", "response"].includes(f.type), line);
    }
    for (const line of readFileSync(join(FIXTURES, "forward_compat.ndjson"), "utf8").split("\n")) {
      if (line.trim()) decodeServerFrame(line);
    }
    assert.equal(decodeServerFrame('{"v":1,"type":"presence_changed","who":"bob"}')!.type, "presence_changed");
    for (const bad of ["not json", "[1]", '{"type":"message"}', '{"v":2,"type":"message"}', '{"v":"1","type":"message"}']) {
      assert.throws(() => decodeServerFrame(bad), ProtocolError, bad);
    }
    assert.equal(decodeServerFrame("   "), null);
  });

  it("a post is read by another member and a handoff keeps its kind and payload", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    await a.post("r", "just words");
    await a.post("r", "see the order", { kind: "work-order", payload: '{"id":7}' });
    const messages = await b.read("r", true);
    assert.deepEqual(messages.map((m) => m.body), ["just words", "see the order"]);
    assert.ok(!("kind" in messages[0]) && !("payload" in messages[0]));
    assert.deepEqual([messages[1].kind, messages[1].payload], ["work-order", '{"id":7}']);
    assert.equal(messages[0].attribution, store.attribution);
    assert.equal((await b.read("r")).length, 2);
    assert.deepEqual(await b.read("r"), []);
  });

  it("history looks back without moving the cursor and pages with since", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    for (let i = 1; i <= 5; i++) await a.post("r", `m${i}`);
    const bodies = (h: { messages: Message[] }) => h.messages.map((m) => m.body);
    const full = await b.history("r");
    assert.deepEqual(bodies(full), ["m1", "m2", "m3", "m4", "m5"]);
    assert.equal(full.more, 0);
    assert.deepEqual(bodies(await b.history("r", { last: 2 })), ["m4", "m5"]);
    const since = full.messages[1].seq;
    assert.deepEqual(bodies(await b.history("r", { last: 2, since })), ["m4", "m5"]);
    assert.deepEqual(bodies(await b.history("r", { since })), ["m3", "m4", "m5"]);
    assert.deepEqual(bodies(await a.history("r", { last: 1 })), ["m5"]);
    assert.equal((await b.read("r")).length, 5);
    await assert.rejects(b.history("r", { last: 0 }), (e: ApiError) => e.code === "invalid");
  });

  it("subscribe replays, catches up, pushes, and resumes after a reconnect", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    for (let i = 0; i < 3; i++) await a.post("r", `m${i}`);
    assert.equal(await b.subscribe("r", 0), 3);
    const seen = await b.collect(true);
    assert.deepEqual(seen.messages.map((m) => m.seq), [1, 2, 3]);
    assert.equal(seen.caughtUp!.seq, 3);
    await a.post("r", "live");
    const live = (await b.nextMessage(5000))!;
    assert.deepEqual([live.seq, live.body], [4, "live"]);
    b.ack("r", 4);
    await b.close();
    await a.post("r", "while away");
    const b2 = await store.client("bob");
    await b2.subscribe("r", 4);
    assert.deepEqual((await b2.collect(true)).messages.map((m) => m.seq), [5]);
  });

  it("a second claim conflicts, names the holder, and a release frees the path", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    await a.claim("r", "src/a.rs", { ttl: 60, reason: "refactor" });
    await assert.rejects(b.claim("r", "src/a.rs"), (e: ApiError) => e.code === "conflict" && e.detail.includes("alice"));
    assert.deepEqual((await b.claims("r")).map((c) => c.owner), ["alice"]);
    await a.release("r", "src/a.rs", "done");
    await b.claim("r", "src/a.rs");
    await assert.rejects(a.release("r", "never/claimed"), (e: ApiError) => e.code === "not_found");
  });

  it("a member without the admin role is refused every act and may prune", async () => {
    const a = await store.client("alice");
    for (const [act, params] of [
      ["run-create", { run: "r1" }],
      ["run-end", { run: "r1" }],
      ["anchor-done", { anchor: "gh:pr:o/r/1" }],
      ["close", { room: "r" }],
    ] as [string, Record<string, unknown>][]) {
      await assert.rejects(a.roleAct(act, params), (e: ApiError) => e.code === "refused" && e.refusals.some((r) => r.rule === "admin-scope"), act);
    }
    await assert.rejects(a.roleAct("reap-anchors"), (e: ApiError) => e.code === "unknown_act");
    await assert.rejects(a.roleAct("run-create"), (e: ApiError) => e.code === "invalid_params");
    assert.equal(await a.prune(), 0);
    await a.post("r", "still here");
    assert.equal(await a.prune(), 0);
    assert.deepEqual((await a.who("r")).map((m) => m.name), ["alice"]);
  });

  it("a direct message reaches one member in a room nobody else may use", async () => {
    const [a, b, c] = [await store.client("alice"), await store.client("bob"), await store.client("carol")];
    const sent = await a.dm("bob", "hello bob");
    assert.deepEqual([sent.room, sent.seq, sent.id.length], ["dm.alice.bob", 1, 26]);
    assert.deepEqual((await b.read(sent.room)).map((m) => [m.sender, m.body]), [["alice", "hello bob"]]);
    assert.equal((await b.dm("alice", "hi back")).seq, 2);
    await assert.rejects(c.read(sent.room), (e: ApiError) => e.code === "refused" && e.refusals.some((r) => r.rule === "dm-members"));
    await assert.rejects(a.dm("alice", "to myself"), (e: ApiError) => e.code === "invalid");
    await assert.rejects(a.dm("bob", "above the ceiling", "secret"), (e: ApiError) => e.code === "refused");
    await assert.rejects(a.request("dm", { to: "bob" }), (e: ApiError) => e.code === "invalid_params");
  });

  it("a member mutes and unmutes a room and read still shows everything", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    await b.post("r", "chatter");
    await a.mute("r");
    assert.deepEqual((await a.read("r", true)).map((m) => m.body), ["chatter"]);
    await a.mute("r", false);
    await a.mute("r", true);
    await assert.rejects(a.mute("bad room"), (e: ApiError) => e.code === "invalid");
    await assert.rejects(a.request("mute", { room: "r" }), (e: ApiError) => e.code === "invalid_params");
  });

  it("an invite is a pointer posted where the member already is", async () => {
    const [a, b] = [await store.client("alice"), await store.client("bob")];
    await a.post("team", "hello");
    await b.post("team", "hi");
    await a.invite("bob", "design", "team");
    const bodies = (await b.read("team")).map((m) => m.body);
    assert.ok(bodies.some((x) => x.includes("klatch join design")), bodies.join("|"));
    await assert.rejects(a.invite("alice", "design", "team"), (e: ApiError) => e.code === "invalid");
    await assert.rejects(a.invite("bob", "bad room", "team"), (e: ApiError) => e.code === "invalid");
    await assert.rejects(a.request("invite", { member: "bob" }), (e: ApiError) => e.code === "invalid_params");
  });

  it("a member reads its rooms, its board and its receipt", async () => {
    const a = await store.client("alice");
    const b = await store.client("bob");
    const sent = await a.post("r", "offering: review help");
    await b.post("r", "hi");
    assert.deepEqual(await a.myRooms(), ["r"]);
    assert.ok((await a.board("r")).includes("review help"));
    assert.deepEqual((await a.receipt("r", sent.seq)).recipients.map((r) => r.name), ["bob"]);
    await assert.rejects(b.receipt("r", sent.seq), (e: ApiError) => e.code === "refused");
  });

  it("a member declares its scope, its kind and its status", async () => {
    const a = await store.client("alice");
    await a.post("r", "here");
    assert.equal(await a.myRole(), null);
    await a.setScope("git:demo-1");
    await a.claim("r", "src/a.rs");
    assert.deepEqual((await a.claims("r")).map((c) => c.scope), ["git:demo-1"]);
    await assert.rejects(a.setScope("not a scope!"), (e: ApiError) => e.code === "invalid");
    await a.setKind("bot");
    assert.deepEqual((await a.who("r")).map((m) => m.kind), ["bot"]);
    await a.setKind(null);
    assert.deepEqual((await a.who("r")).map((m) => m.kind), [undefined]);
    await assert.rejects(a.setKind("session"), (e: ApiError) => e.code === "invalid");
    await a.setStatus("r", "done");
    assert.deepEqual((await a.who("r")).map((m) => m.status), ["done"]);
    await assert.rejects(a.setStatus("r", ""), (e: ApiError) => e.code === "invalid");
    await a.unsubscribe("r");
  });

  it("a member that is not the reviewer is refused both review methods", async () => {
    const a = await store.client("alice");
    await assert.rejects(a.reviewNext(), (e: ApiError) => {
      assert.equal(e.code, "refused");
      assert.ok(e.refusals.some((r) => r.rule === "reviewer"));
      return true;
    });
    await assert.rejects(a.reviewVerdict(1, "allow"), (e: ApiError) => e.code === "refused");
    await assert.rejects(a.request("review_verdict", { id: 1 }), (e: ApiError) => e.code === "invalid_params");
  });

  it("a refusal carries its rule and never repeats the text", async () => {
    const a = await store.client("alice");
    const secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    await assert.rejects(a.post("r", `token ${secret}`), (e: ApiError) => {
      assert.equal(e.code, "refused");
      assert.ok(e.refusals.some((r) => r.rule === "credentials"));
      assert.ok(!(JSON.stringify(e.refusals) + e.detail).includes(secret));
      return true;
    });
  });

  it("bad requests get a failed response and the session carries on", async () => {
    const a = await store.client("alice");
    await assert.rejects(a.request("no_such_method"), (e: ApiError) => e.code === "unknown_method");
    await assert.rejects(a.request("post", { room: "r" }), (e: ApiError) => e.code === "invalid_params");
    await assert.rejects(a.post("bad room name", "x"), (e: ApiError) => e.code === "invalid");
    await assert.rejects(a.post("r", "p", { payload: "needs a kind" }), ApiError);
    assert.equal((await a.post("r", "still working")).seq, 1);
  });

  it("closing the input ends the session cleanly", async () => {
    const a = await store.client("alice");
    await a.post("r", "x");
    assert.equal(await a.close(), 0);
  });
});

describe("klatch client socket connect", { skip: !daemonBuilt }, () => {
  let store = new DaemonStore();
  afterEach(async () => {
    await store.close();
    store = new DaemonStore();
  });
  after(async () => store.close());

  it("a typed name is the member and unattributed after the handshake", async () => {
    const a = await store.client("alice");
    assert.deepEqual([a.member, a.attribution, a.clearance], ["alice", "unattributed", "public"]);
    await a.post("r", "hi");
    assert.equal((await a.history("r")).messages[0].sender, "alice");
  });

  it("no identity is the unmapped-process the daemon gives every local process", async () => {
    await store.start();
    const c = await Client.connectSocket(store.sock);
    store.clients.push(c);
    assert.deepEqual([c.member, c.attribution], ["unmapped-process", "unattributed"]);
  });

  it("a refused identity rejects without echoing the name", async () => {
    await store.start();
    for (const bad of ["admin", "a b", "x/y"]) {
      await assert.rejects(Client.connectSocket(store.sock, { name: bad }), (e: ApiError) => {
        assert.equal(e.code, "identity.refused");
        assert.ok(!e.detail.includes(bad), e.detail);
        return true;
      });
    }
  });

  it("a name with a credential, or a member without a token, is refused before anything is sent", async () => {
    await assert.rejects(Client.connectSocket(store.sock, { name: "alice", member: "w-1", token: "t" }), TypeError);
    await assert.rejects(Client.connectSocket(store.sock, { member: "w-1" }), TypeError);
  });

  it("a credential that is not live is refused and the token is never echoed", async () => {
    await store.start();
    const token = "0".repeat(64);
    await assert.rejects(Client.connectSocket(store.sock, { member: "w-1", token }), (e: ApiError) => {
      assert.equal(e.code, "identity.refused");
      assert.ok(!e.message.includes(token));
      return true;
    });
  });

  it("a socket nobody listens on rejects, and close answers zero once the daemon ends the session", async () => {
    await assert.rejects(Client.connectSocket(join(store.sockdir, "nobody.sock"), { name: "alice" }));
    const a = await store.client("alice");
    assert.equal(await a.close(), 0);
    assert.equal(await a.close(), 0); // closing again is harmless
    const b = await store.client("bob");
    assert.equal((await b.post("r", "after")).seq, 1);
  });

  it("the daemon ending the session is a protocol error, not a hang", async () => {
    const a = await store.client("alice");
    await a.post("r", "x");
    store.stopDaemon();
    await assert.rejects(a.post("r", "y"));
  });
});

// On Linux a peer that dies surfaces as ECONNRESET on the socket, not a clean end. That must end the session (pending
// and later requests reject with a ProtocolError), never become an uncaught exception.
describe("klatch client socket reset", () => {
  it("a connection reset by the peer ends the session without an uncaught error", async () => {
    const dir = mkdtempSync(join("/tmp", "kr-"));
    const path = join(dir, "r.sock");
    const server = createServer((c) => {
      c.on("error", () => {});
      c.write('{"v":1,"type":"hello","member":"unmapped-process","attribution":"unattributed","clearance":"public"}\n');
    });
    await new Promise<void>((r) => server.listen(path, r));
    try {
      const c = await Client.connectSocket(path);
      // what Linux reports for a peer that died: an error event on the socket (Node cannot reset a Unix socket from
      // the other end, so the event is raised here, on every platform)
      const socket = (c as unknown as { socket: { destroy(e: Error): void } }).socket;
      socket.destroy(Object.assign(new Error("read ECONNRESET"), { code: "ECONNRESET" }));
      await assert.rejects(c.post("r", "x"), ProtocolError);
      await sleep(100); // an error event with no listener would be thrown here
      await assert.rejects(c.post("r", "y"), ProtocolError);
    } finally {
      server.close();
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
