// The conformance scenario for a klatch client (#119): the same steps as the Python and Rust clients, against the
// real gateway. Run from this directory:
//   KLATCH_BIN=/path/to/klatch node --experimental-strip-types --test conformance.test.ts
import assert from "node:assert/strict";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { after, afterEach, describe, it } from "node:test";
import { fileURLToPath } from "node:url";
import { ApiError, Client, decodeServerFrame, PROTOCOL_VERSION, ProtocolError } from "./klatch.ts";

const here = dirname(fileURLToPath(import.meta.url));
const BIN = process.env.KLATCH_BIN || resolve(here, "../../proto/target/debug/klatch");
const FIXTURES = resolve(here, "../../proto/klatch/fixtures/protocol");

class Store {
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

describe("klatch client conformance", () => {
  let store = new Store();
  afterEach(async () => {
    await store.close();
    store = new Store();
  });
  after(async () => store.close());

  it("speaks protocol version one", () => assert.equal(PROTOCOL_VERSION, 1));

  it("hello says who the gateway believes you are", async () => {
    const a = await store.client("alice");
    assert.deepEqual([a.member, a.attribution, a.clearance], ["alice", "unattributed", "public"]);
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
    assert.equal(messages[0].attribution, "unattributed");
    assert.equal((await b.read("r")).length, 2);
    assert.deepEqual(await b.read("r"), []);
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
