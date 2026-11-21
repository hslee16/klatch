// The HTTP and server-sent-events transport of the TypeScript client (#1114) for `klatch-server http` (#1059). Run from
// this directory: node --experimental-strip-types --test http.test.ts
//
// The success path runs against a small in-process server that speaks the same wire (`GET /v1/stream` as server-sent
// events with `X-Klatch-Session`, `POST /v1/frames` with that header), because a verified connection needs a worker
// credential and only the human issues one. The refusal path also runs against the real `klatch-server http` when it
// is built. Every wait has a deadline, no test reads a stream to its end, and every error is injected, not provoked
// from the platform (a peer that dies is ECONNRESET on Linux and a clean end on macOS).
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, openSync, readFileSync, rmSync } from "node:fs";
import { createServer, type IncomingMessage, type Server, type ServerResponse } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { afterEach, beforeEach, describe, it } from "node:test";
import { setTimeout as sleep } from "node:timers/promises";
import { fileURLToPath } from "node:url";
import { ApiError, Client, ProtocolError } from "./klatch.ts";

const here = dirname(fileURLToPath(import.meta.url));
const SERVER_BIN = process.env.KLATCH_SERVER_BIN || resolve(here, "../../proto/target/debug/klatch-server");
const TOKEN = "tok-secret-1";
const WAIT = 5_000;

type Msg = { v: 1; type: "message"; room: string; seq: number; id: string; sender: string; attribution: string; label: string; body: string; ts: number };
type Recorded = { method: string; url: string; headers: IncomingMessage["headers"]; body: string };

function sse(frame: Record<string, unknown>): string {
  let head = `event: ${frame.type}\n`;
  if (frame.type === "message") head += `id: ${frame.room}:${frame.seq}\n`;
  return `${head}data: ${JSON.stringify(frame)}\n\n`;
}

/** Poll `cond` until it holds, with a deadline. */
async function until(cond: () => boolean, ms = WAIT): Promise<void> {
  const end = Date.now() + ms;
  while (!cond()) {
    if (Date.now() > end) throw new Error("the condition never held");
    await sleep(10);
  }
}

class FakeServer {
  member = "w-1";
  rooms = new Map<string, Msg[]>();
  sessions = new Map<string, { res: ServerResponse; subs: Set<string> }>();
  requests: Recorded[] = [];
  postsRefused = 0;
  opened = 0;
  port = 0;
  private http: Server;

  constructor() {
    this.http = createServer((req, res) => this.route(req, res));
  }

  async listen(): Promise<void> {
    await new Promise<void>((r) => this.http.listen(0, "127.0.0.1", r));
    this.port = (this.http.address() as { port: number }).port;
  }

  get url() {
    return `http://127.0.0.1:${this.port}`;
  }

  private authorized(req: IncomingMessage) {
    return req.headers.authorization === `Bearer ${TOKEN}` && req.headers["x-klatch-member"] === this.member;
  }

  private reply(res: ServerResponse, status: number, body = "") {
    res.writeHead(status, { "Content-Length": Buffer.byteLength(body), Connection: "close" });
    res.end(body);
  }

  private route(req: IncomingMessage, res: ServerResponse) {
    const chunks: Buffer[] = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      const body = Buffer.concat(chunks).toString();
      this.requests.push({ method: req.method ?? "", url: req.url ?? "", headers: req.headers, body });
      if (!this.authorized(req)) return this.reply(res, 401, "unauthorized");
      if (req.method === "GET" && req.url === "/v1/stream") return this.stream(res);
      if (req.method === "POST" && req.url === "/v1/frames") return this.frames(req, res, body);
      this.reply(res, 404, "not found");
    });
  }

  private stream(res: ServerResponse) {
    const id = `s${++this.opened}`;
    res.writeHead(200, { "Content-Type": "text/event-stream", "X-Klatch-Session": id, Connection: "close" });
    this.sessions.set(id, { res, subs: new Set() });
    res.on("close", () => this.sessions.delete(id));
    res.write(sse({ v: 1, type: "hello", member: this.member, attribution: "verified", clearance: "public" }));
    res.write(": a comment line the client ignores\n\n");
  }

  private frames(req: IncomingMessage, res: ServerResponse, body: string) {
    const session = this.sessions.get(String(req.headers["x-klatch-session"]));
    if (!session) return this.reply(res, 404, "no such session");
    if (this.postsRefused > 0) {
      this.postsRefused--;
      return this.reply(res, 429, "the session is busy; retry");
    }
    for (const line of body.split("\n").filter((l) => l.trim())) this.handle(session, JSON.parse(line));
    this.reply(res, 202);
  }

  private message(room: string, seq: number, body: string): Msg {
    return { v: 1, type: "message", room, seq, id: `m${seq}`, sender: "other", attribution: "verified", label: "public", body, ts: seq };
  }

  private handle(session: { res: ServerResponse; subs: Set<string> }, frame: any) {
    if (frame.type !== "request") return;
    const { method, params, id } = frame;
    let result: Record<string, unknown> = {};
    let replay: Msg[] = [];
    if (method === "post") {
      const msgs = this.rooms.get(params.room) ?? [];
      this.rooms.set(params.room, msgs);
      msgs.push(this.message(params.room, msgs.length + 1, params.body));
      result = { seq: msgs.length, id: `m${msgs.length}` };
      for (const s of this.sessions.values()) if (s.subs.has(params.room)) s.res.write(sse(msgs[msgs.length - 1]));
    } else if (method === "subscribe") {
      session.subs.add(params.room);
      const msgs = this.rooms.get(params.room) ?? [];
      this.rooms.set(params.room, msgs);
      result = { head: msgs.length };
      replay = msgs.filter((m) => m.seq > params.from_seq);
    } else if (method === "unsubscribe") {
      session.subs.delete(params.room);
    }
    session.res.write(sse({ v: 1, type: "response", id, ok: true, result }));
    if (method === "subscribe") {
      for (const m of replay) session.res.write(sse(m));
      session.res.write(sse({ v: 1, type: "caught_up", room: params.room, head: result.head }));
    }
  }

  say(room: string, body: string) {
    const msgs = this.rooms.get(room) ?? [];
    this.rooms.set(room, msgs);
    msgs.push(this.message(room, msgs.length + 1, body));
    for (const s of this.sessions.values()) if (s.subs.has(room)) s.res.write(sse(msgs[msgs.length - 1]));
  }

  /** End every open stream the way a peer that goes away does (a clean end of the body). */
  dropStreams() {
    for (const s of this.sessions.values()) s.res.end();
  }

  subscribes(): Record<string, unknown>[] {
    const out: Record<string, unknown>[] = [];
    for (const r of this.requests) {
      if (r.method !== "POST") continue;
      for (const line of r.body.split("\n").filter((l) => l.trim())) {
        const f = JSON.parse(line);
        if (f.method === "subscribe") out.push(f.params);
      }
    }
    return out;
  }

  async close() {
    this.dropStreams();
    this.http.closeAllConnections();
    await new Promise<void>((r) => this.http.close(() => r()));
  }
}

describe("klatch client over HTTP and server-sent events", () => {
  let server: FakeServer;
  let clients: Client[];
  beforeEach(async () => {
    server = new FakeServer();
    await server.listen();
    clients = [];
  });
  afterEach(async () => {
    for (const c of clients) await c.close();
    await server.close();
  });

  async function connect(opts: Record<string, unknown> = {}): Promise<Client> {
    const c = await Client.connectHttp(server.url, { member: "w-1", token: TOKEN, ...opts });
    clients.push(c);
    return c;
  }

  it("reads the hello first, and who you are comes from the credential", async () => {
    const c = await connect();
    assert.deepEqual([c.member, c.attribution, c.clearance], ["w-1", "verified", "public"]);
    const get = server.requests[0];
    assert.deepEqual([get.method, get.url], ["GET", "/v1/stream"]);
    assert.equal(get.headers.authorization, `Bearer ${TOKEN}`);
    assert.equal(get.headers["x-klatch-member"], "w-1");
  });

  it("sends a request as a POST with the session header and gets the response on the stream", async () => {
    const c = await connect();
    assert.equal((await c.post("lobby", "hello")).seq, 1);
    const post = server.requests.find((r) => r.method === "POST")!;
    assert.equal(post.url, "/v1/frames");
    assert.equal(post.headers["x-klatch-session"], "s1");
    assert.equal(post.headers.authorization, `Bearer ${TOKEN}`);
    const frame = JSON.parse(post.body);
    assert.deepEqual([frame.v, frame.type, frame.method], [1, "request", "post"]);
  });

  it("gives history then live messages with the server's sequence numbers", async () => {
    server.say("lobby", "one");
    server.say("lobby", "two");
    const c = await connect();
    await c.subscribe("lobby", 0);
    const got = await c.collect(true, WAIT);
    assert.deepEqual(got.messages.map((m) => m.seq), [1, 2]);
    assert.ok(got.caughtUp);
    server.say("lobby", "three");
    assert.equal((await c.nextMessage(WAIT))?.seq, 3);
  });

  it("ignores a comment line", async () => {
    const c = await connect();
    assert.equal((await c.post("lobby", "x")).seq, 1);
  });

  it("reconnects after a dropped stream and resubscribes from the last seq, without a repeat", async () => {
    const c = await connect();
    await c.subscribe("lobby", 0);
    await c.post("lobby", "one");
    await c.post("lobby", "two");
    assert.deepEqual([(await c.nextMessage(WAIT))?.seq, (await c.nextMessage(WAIT))?.seq], [1, 2]);
    server.dropStreams();
    await until(() => server.subscribes().length >= 2);
    assert.deepEqual(server.subscribes()[1], { room: "lobby", from_seq: 2 });
    assert.equal(server.opened, 2);
    server.say("lobby", "three");
    assert.equal((await c.nextMessage(WAIT))?.seq, 3);
    assert.equal(await c.nextMessage(300), null);
    assert.equal((await c.post("lobby", "four")).seq, 4);
  });

  it("does not resubscribe a room that was unsubscribed", async () => {
    const c = await connect();
    await c.subscribe("lobby", 0);
    await c.unsubscribe("lobby");
    server.dropStreams();
    await until(() => server.opened >= 2);
    assert.equal(server.subscribes().length, 1);
  });

  it("treats an error injected into the stream as a dropped stream, not a crash", async () => {
    const c = await connect();
    await c.subscribe("lobby", 0);
    await c.post("lobby", "one");
    assert.equal((await c.nextMessage(WAIT))?.seq, 1);
    // what Linux reports for a peer that died: an error event on the stream (raised here, on every platform)
    const stream = (c as unknown as { httpStream: { destroy(e: Error): void } }).httpStream;
    stream.destroy(Object.assign(new Error("read ECONNRESET"), { code: "ECONNRESET" }));
    await until(() => server.opened >= 2);
    assert.deepEqual(server.subscribes()[1], { room: "lobby", from_seq: 1 });
    server.say("lobby", "two");
    assert.equal((await c.nextMessage(WAIT))?.seq, 2);
    assert.equal(await c.nextMessage(300), null);
  });

  it("ends the session with a ProtocolError when the server stays gone", async () => {
    const c = await connect({ reconnectAttempts: 2, reconnectDelayMs: 10 });
    await server.close();
    await assert.rejects(c.post("lobby", "x"), ProtocolError);
  });

  it("retries a busy session, and a post to an ended session is a ProtocolError", async () => {
    const c = await connect();
    server.postsRefused = 2;
    assert.equal((await c.post("lobby", "x")).seq, 1);
    server.sessions.clear();
    await assert.rejects(c.post("lobby", "y"), ProtocolError);
  });

  it("answers identity.refused for a bad credential and never repeats the token", async () => {
    await assert.rejects(Client.connectHttp(server.url, { member: "w-1", token: "wrong-token-value" }), (e: ApiError) => {
      assert.ok(e instanceof ApiError);
      assert.equal(e.code, "identity.refused");
      assert.ok(!String(e).includes("wrong-token-value"));
      return true;
    });
    await assert.rejects(Client.connectHttp(server.url, { member: "someone-else", token: TOKEN }), (e: ApiError) => {
      assert.equal(e.code, "identity.refused");
      assert.ok(!String(e).includes(TOKEN));
      return true;
    });
  });

  it("refuses a member without a token, or the reverse, before anything is sent", async () => {
    for (const id of [{ member: "w-1" }, { token: "t" }, {}]) {
      await assert.rejects(Client.connectHttp(server.url, id), TypeError);
    }
    assert.equal(server.requests.length, 0);
  });

  it("accepts only a plain http loopback url", async () => {
    for (const url of ["https://127.0.0.1:1", "http://example.com:80", "ftp://127.0.0.1:1", "127.0.0.1:1"]) {
      await assert.rejects(Client.connectHttp(url, { member: "w-1", token: TOKEN }), TypeError, url);
    }
    assert.equal(server.requests.length, 0);
  });

  it("rejects when nothing listens on the port", async () => {
    const port = server.port;
    await server.close();
    await assert.rejects(Client.connectHttp(`http://127.0.0.1:${port}`, { member: "w-1", token: TOKEN, connectTimeoutMs: 2000 }));
  });

  it("closes the session and answers zero, and a close is not a reconnect", async () => {
    const c = await connect();
    assert.equal(await c.close(), 0);
    await until(() => server.sessions.size === 0);
    assert.equal(server.opened, 1);
    assert.equal(await c.close(), 0);
  });
});

const built = existsSync(SERVER_BIN);
describe("klatch client over HTTP against the real server", { skip: !built }, () => {
  it("a credential the store does not know is identity.refused", async () => {
    const home = mkdtempSync(join(tmpdir(), "klatch-http-ts-"));
    const log = join(home, "err.log");
    const env = { PATH: "/usr/bin:/bin", HOME: home, KLATCH_HOME: home, KLATCH_KEY_DIR: join(home, "keys") };
    const proc = spawn(SERVER_BIN, ["http", "--listen", "127.0.0.1:0"], { env, stdio: ["ignore", "ignore", openSync(log, "w")] });
    try {
      let port = 0;
      const end = Date.now() + 30_000;
      while (port === 0 && Date.now() < end) {
        const m = /http listening on 127\.0\.0\.1:(\d+)/.exec(readFileSync(log, "utf8"));
        if (m) port = Number(m[1]);
        else await sleep(20);
      }
      assert.ok(port > 0, "the server never listened");
      await assert.rejects(
        Client.connectHttp(`http://127.0.0.1:${port}`, { member: "w-1", token: "not-a-live-token", connectTimeoutMs: 10_000 }),
        (e: ApiError) => {
          assert.equal(e.code, "identity.refused");
          assert.ok(!String(e).includes("not-a-live-token"));
          return true;
        },
      );
    } finally {
      proc.kill("SIGKILL");
      rmSync(home, { recursive: true, force: true });
    }
  });
});
