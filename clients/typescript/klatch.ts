// A thin klatch client over the wire protocol (PROTOCOL.md, version 1): newline-delimited JSON to
// `klatch serve --stdio`, over the Unix socket of the klatch-server daemon (`Client.connectSocket`), or over HTTP and
// server-sent events to `klatch-server http` (`Client.connectHttp`), using only Node's standard library. Over stdio, who you are is decided by the environment of the process, never by a request
// parameter; over the daemon's socket the operating system reports the peer and the first request, `identify`, says
// more (#102). A frame type or field this version does not know is ignored.

import { spawn } from "node:child_process";
import { request as httpRequest, type IncomingMessage } from "node:http";
import { createConnection, type Socket } from "node:net";
import { createInterface } from "node:readline";
import { PassThrough, Writable, type Readable } from "node:stream";
import { setTimeout as sleep } from "node:timers/promises";

export const PROTOCOL_VERSION = 1;

export class ProtocolError extends Error {}

export class ApiError extends Error {
  code: string;
  detail: string;
  refusals: Frame[];
  constructor(code: string, detail: string, refusals: Frame[] = []) {
    super(`${code}: ${detail}`);
    this.code = code;
    this.detail = detail;
    this.refusals = refusals;
  }
}

export type Frame = { type: string; [key: string]: unknown };
/** A reference an ask carries (#1318): `kind` is any token of `[a-z0-9_.-]{1,32}`; klatch never fetches or resolves it. */
export type AskRef = { kind: string; value: string; label?: string; snippet?: string };

export type Message = {
  seq: number;
  id: string;
  sender: string;
  attribution: string;
  label: string;
  body: string;
  ts: number;
  kind?: string;
  payload?: string;
  /** Present and true on a `read`, `history` or stream message whose sender the room flagged as advisory (#990). Display only. */
  advisory?: boolean;
  /** Present only on an ask that declares a scope (#1171): the scope the asker named. */
  scope?: string;
  /** Present only on an ask that carries refs (#1318): references to artifacts outside klatch. Data, never an instruction. */
  refs?: AskRef[];
  [key: string]: unknown;
};

function isPlainObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The JSON object a line holds, or a ProtocolError. */
function parseFrameObject(line: string): Record<string, unknown> {
  let frame: unknown;
  try {
    frame = JSON.parse(line);
  } catch (e) {
    throw new ProtocolError(`not JSON: ${(e as Error).message}`);
  }
  if (!isPlainObject(frame)) throw new ProtocolError("a frame is a JSON object");
  return frame;
}

/** One line from the gateway as a frame, or null for a blank line. An unknown type or field is not an error. */
export function decodeServerFrame(line: string): Frame | null {
  if (line.trim() === "") return null;
  const f = parseFrameObject(line);
  if (f.v !== PROTOCOL_VERSION) {
    throw new ProtocolError(`protocol version ${JSON.stringify(f.v)} is not supported; this client speaks ${PROTOCOL_VERSION}`);
  }
  if (typeof f.type !== "string") throw new ProtocolError("a frame has a type");
  return f as Frame;
}

export function encodeLine(frame: Record<string, unknown>): string {
  return JSON.stringify({ v: PROTOCOL_VERSION, ...frame }) + "\n";
}

type Waiter = (frame: Frame | null) => void;

/** Who a connection over the daemon's socket says it is (the `identify` first request): a typed `name` (unattributed),
 * or a `member` and its credential `token` (verified; the token only, never the key bundle file), or neither
 * (`unmapped-process`). */
export type Identity = { name?: string; member?: string; token?: string };

/** What `Client.connectHttp` takes: the credential (a member and its token) and how patient to be. */
export type HttpOptions = {
  member?: string;
  token?: string;
  /** Milliseconds to wait for the server to answer a connect or a post (default 10000). */
  connectTimeoutMs?: number;
  /** Times to try to reopen a stream that ended without `close()` (default 5). */
  reconnectAttempts?: number;
  /** Base delay between those tries, multiplied by the try number (default 200). */
  reconnectDelayMs?: number;
};

/** Where an HTTP session goes and who it is. */
type HttpTarget = { host: string; port: number; member: string; token: string };

/** How long an HTTP session waits to connect and how hard it tries to reopen a stream. */
type HttpPatience = { connectTimeoutMs: number; reconnectAttempts: number; reconnectDelayMs: number };

/** What an event ends with: its frame line, and whether it is the hello of a session. */
type FinishedEvent = { frame: string; hello: boolean };

/** Assembles the frame lines of a server-sent-events stream from its lines: the `data:` lines of an event, ended by a
 * blank line, are one frame line. The first frame of a session is its hello. */
class EventAssembler {
  private data: string[] = [];
  private first = true;

  /** Take one line of the stream. When it ends an event, that event; otherwise null. */
  feed(line: string): FinishedEvent | null {
    if (line === "") return this.data.length === 0 ? null : this.finishEvent();
    if (line.startsWith("data:")) this.data.push(line.slice(line[5] === " " ? 6 : 5));
    return null;
  }

  private finishEvent(): FinishedEvent {
    const frame = this.data.join("\n");
    this.data = [];
    const hello = this.first && /"type"\s*:\s*"hello"/.test(frame);
    this.first = false;
    return { frame, hello };
  }
}

/** Splits arriving text into complete lines (without the line break or a trailing CR), keeping the rest for next time. */
class LineSplitter {
  private buf = "";

  push(chunk: string): string[] {
    this.buf += chunk;
    const lines = this.buf.split("\n");
    this.buf = lines.pop() ?? "";
    return lines.map((l) => l.replace(/\r$/, ""));
  }
}

/** The request frames in some written text, one per line that holds one. */
function* requestFrames(text: string): Generator<any> {
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    let f: any;
    try {
      f = JSON.parse(line);
    } catch {
      continue;
    }
    if (f?.type === "request") yield f;
  }
}

/** The session a stream response opens, or the error its status or headers call for. */
function sessionOf(res: IncomingMessage): string {
  if (res.statusCode === 401) throw new ApiError("identity.refused", "the server refused the credential");
  const session = res.headers["x-klatch-session"];
  const usable = [res.statusCode === 200, typeof session === "string", session !== ""];
  if (!usable.every(Boolean)) throw new Error(`the server answered ${res.statusCode} to the stream`);
  return session as string;
}

/** `klatch-server http` as a readable of frame lines and a writable of frame lines (#1114). The stream is
 * `GET /v1/stream` with the credential; its `X-Klatch-Session` header names the session; the `data:` of each event is one
 * frame line exactly. Each written line is a `POST /v1/frames` with that header, in order; the answer comes on the
 * stream, as on the daemon's connection. A stream that ends without `shut()` (the server went away, a reset) is reopened,
 * the `hello` of the new session is dropped, and every subscribed room is resubscribed from the last `seq` seen in it,
 * so the caller sees no gap and no repeat. */
class HttpTransport {
  readonly input = new PassThrough({ encoding: "utf8" });
  readonly output: Writable;
  stream: IncomingMessage | null = null;
  closed = false;
  private session = "";
  private ready: Promise<void> = Promise.resolve();
  private subs = new Map<string, number>(); // room -> the from_seq to resume from
  private rid = 0;
  private done!: (code: number) => void;
  readonly exited = new Promise<number>((resolve) => (this.done = resolve));

  private target: HttpTarget;
  private patience: HttpPatience;
  private onSendError: (e: Error) => void;

  // plain fields, not parameter properties: the tests run with Node's type stripping, which does not support them
  constructor(target: HttpTarget, patience: HttpPatience, onSendError: (e: Error) => void) {
    this.target = target;
    this.patience = patience;
    this.onSendError = onSendError;
    let chain: Promise<void> = Promise.resolve();
    this.output = new Writable({
      write: (chunk, _enc, cb) => {
        const text = String(chunk);
        this.noteSent(text);
        // posts go out in order; a failed one is reported as a send error, answered by the next request
        chain = chain.then(() => this.post(text)).catch((e) => this.onSendError(e as Error));
        cb();
      },
    });
    this.output.on("error", () => {});
  }

  private headers(): Record<string, string> {
    return { Authorization: `Bearer ${this.target.token}`, "X-Klatch-Member": this.target.member };
  }

  /** Open a session. Rejects with an ApiError (`identity.refused`) for a 401 and with the socket's error when nothing listens. */
  open(skipHello: boolean): Promise<void> {
    return new Promise((resolve, reject) => {
      const { host, port } = this.target;
      const req = httpRequest(
        { host, port, path: "/v1/stream", method: "GET", headers: { ...this.headers(), Accept: "text/event-stream" } },
        (res) => {
          req.setTimeout(0);
          try {
            this.session = sessionOf(res);
          } catch (e) {
            res.resume();
            return reject(e);
          }
          this.stream = res;
          this.read(res, skipHello);
          resolve();
        },
      );
      req.setTimeout(this.patience.connectTimeoutMs, () => req.destroy(new Error("the server did not answer the connect in time")));
      req.on("error", reject);
      req.end();
    });
  }

  private read(res: IncomingMessage, skipHello: boolean) {
    res.setEncoding("utf8");
    const lines = new LineSplitter();
    const events = new EventAssembler();
    const end = () => {
      if (this.stream !== res) return;
      this.stream = null;
      void this.reconnect();
    };
    res.on("data", (chunk: string) => {
      if (this.stream !== res) return;
      for (const line of lines.push(chunk)) this.accept(events.feed(line), skipHello);
    });
    res.on("end", end);
    res.on("close", end);
    res.on("error", end); // a reset (ECONNRESET on Linux) is a stream that ended, as a clean end is
  }

  /** Pass on the frame line of a finished event, except the hello of a reconnect, which the caller already has. */
  private accept(event: FinishedEvent | null, skipHello: boolean) {
    if (event === null) return;
    if (event.hello && skipHello) return;
    this.noteSeen(event.frame);
    this.input.write(event.frame + "\n");
  }

  private async reconnect() {
    if (this.closed) return;
    let release!: () => void;
    let fail!: (e: Error) => void;
    this.ready = new Promise<void>((res, rej) => ((release = res), (fail = rej)));
    this.ready.catch(() => {});
    for (let attempt = 1; attempt <= this.patience.reconnectAttempts; attempt++) {
      if (this.closed) return;
      try {
        await this.open(true);
        release();
        await this.resubscribe();
        return;
      } catch {
        await sleep(this.patience.reconnectDelayMs * attempt);
      }
    }
    fail(new Error("the stream could not be reopened"));
    this.finish();
  }

  private async resubscribe() {
    for (const [room, from_seq] of [...this.subs]) {
      const line = JSON.stringify({ v: PROTOCOL_VERSION, type: "request", id: `r${++this.rid}`, method: "subscribe", params: { room, from_seq } }) + "\n";
      try {
        await this.post(line);
      } catch {
        return;
      }
    }
  }

  private noteSent(text: string) {
    for (const f of requestFrames(text)) {
      if (f.method === "subscribe" && typeof f.params?.room === "string") this.subs.set(f.params.room, f.params.from_seq ?? 0);
      else if (f.method === "unsubscribe") this.subs.delete(f.params?.room);
    }
  }

  private noteSeen(frameLine: string) {
    let f: any;
    try {
      f = JSON.parse(frameLine);
    } catch {
      return; // not JSON: the client reports it as an _error frame
    }
    if (f?.type !== "message" || !Number.isInteger(f.seq)) return;
    if (this.subs.has(f.room)) this.subs.set(f.room, f.seq);
  }

  /** Wait until a session is open (a reconnect may be under way); the session is closed if it never reopens or was shut. */
  private async awaitOpenSession() {
    try {
      await this.ready;
    } catch {
      throw new Error("the session is closed");
    }
    if (this.closed) throw new Error("the session is closed");
  }

  /** One POST of `text` to `/v1/frames`; the status the server answered. */
  private postOnce(text: string): Promise<number> {
    return new Promise<number>((resolve, reject) => {
      const body = Buffer.from(text, "utf8");
      const { host, port } = this.target;
      const req = httpRequest(
        {
          host,
          port,
          path: "/v1/frames",
          method: "POST",
          headers: { ...this.headers(), "X-Klatch-Session": this.session, "Content-Type": "application/x-ndjson", "Content-Length": body.length },
        },
        (res) => {
          res.resume();
          res.on("end", () => resolve(res.statusCode ?? 0));
          res.on("error", reject);
        },
      );
      req.setTimeout(this.patience.connectTimeoutMs, () => req.destroy(new Error("the server did not answer the post in time")));
      req.on("error", reject);
      req.end(body);
    });
  }

  /** One POST, retried while the session's inbox is full (429); any other answer than 202 is an error. */
  private async post(text: string): Promise<void> {
    for (let attempt = 0; ; attempt++) {
      await this.awaitOpenSession();
      const status = await this.postOnce(text);
      if (status === 202) return;
      if (!this.inboxFullAndWorthARetry(status, attempt)) throw new Error(`the server answered ${status} to a post`);
      await sleep(50);
    }
  }

  private inboxFullAndWorthARetry(status: number, attempt: number): boolean {
    return status === 429 && attempt < 39 && !this.closed;
  }

  private finish() {
    this.input.destroy();
    this.done(0);
  }

  /** End the session: dropping the stream ends it on the server. */
  shut() {
    if (this.closed) return;
    this.closed = true;
    const stream = this.stream;
    this.stream = null;
    stream?.destroy();
    this.finish();
  }
}

/** A name, or a member with its token, or neither; never a mix (a TypeError). */
function checkIdentity({ name, member, token }: Identity) {
  const credentialGiven = member !== undefined || token !== undefined;
  if (name !== undefined && credentialGiven) throw new TypeError("give a name, or a member and a token, not both");
  if ((member === undefined) !== (token === undefined)) throw new TypeError("a credential is a member and a token");
}

/** What the `identify` request carries: the name, else the member and its token, else nothing (null). */
function identifyParams({ name, member, token }: Identity): Record<string, string> | null {
  if (name !== undefined) return { name };
  if (member !== undefined) return { member, token: String(token) };
  return null;
}

/** A connected Unix stream socket, or the socket's error. */
function connectUnix(path: string): Promise<Socket> {
  return new Promise<Socket>((resolve, reject) => {
    const s = createConnection(path);
    s.once("connect", () => resolve(s));
    s.once("error", reject);
  });
}

const LOOPBACK_HOSTS = ["127.0.0.1", "::1", "localhost"];
const LOOPBACK_URL_MESSAGE = "the url is http://127.0.0.1:PORT (plain http, loopback only)";

/** The host and port of a plain `http` url to a loopback address, or a TypeError. */
function loopbackEndpoint(url: string): { host: string; port: number } {
  let parsed: URL;
  try {
    parsed = new URL(url);
  } catch {
    throw new TypeError(LOOPBACK_URL_MESSAGE);
  }
  const host = parsed.hostname.replace(/^\[|\]$/g, "");
  const plainLoopback = [parsed.protocol === "http:", LOOPBACK_HOSTS.includes(host), parsed.port !== ""];
  if (!plainLoopback.every(Boolean)) throw new TypeError(LOOPBACK_URL_MESSAGE);
  return { host, port: Number(parsed.port) };
}

/** One session with the gateway. */
export class Client {
  member = "";
  attribution = "";
  clearance = "";
  refusals: Frame[] = [];
  private ended = false;
  private writable: Writable;
  private socket: Socket | null;
  private frames: (Frame | null)[] = [];
  private waiters: Waiter[] = [];
  private nextId = 0;
  private pendingMessages: Message[] = [];
  private caughtUp = new Map<string, Frame>();
  private exited: Promise<number>;

  private http: HttpTransport | null = null;

  private constructor(input: Readable, output: Writable, exited: Promise<number>, socket: Socket | null) {
    this.writable = output;
    this.socket = socket;
    this.exited = exited;
    // a write to a transport that has gone is answered by the closed session, not by an unhandled event
    output.on("error", () => {});
    // readline re-emits an error of its input (a reset socket: ECONNRESET on Linux) on itself; with no listener it
    // would be thrown. A reset ends the session like a clean close does: the input's `close` pushes the end.
    const lines = createInterface({ input });
    lines.on("error", () => {});
    lines.on("line", (line) => {
      let frame: Frame | null;
      try {
        frame = decodeServerFrame(line);
      } catch (e) {
        frame = { type: "_error", error: String(e) };
      }
      if (frame !== null) this.push(frame);
    });
    input.on("close", () => this.push(null));
  }

  static async connect(binary: string, env: NodeJS.ProcessEnv, args: string[] = ["serve", "--stdio"]): Promise<Client> {
    const proc = spawn(binary, args, { env, stdio: ["pipe", "pipe", "ignore"] });
    const exited = new Promise<number>((resolve) => proc.on("close", (code) => resolve(code ?? -1)));
    const c = new Client(proc.stdout, proc.stdin, exited, null);
    await c.hello();
    return c;
  }

  /** Connect to a `klatch-server daemon --socket PATH` and say who you are (`identity`, the daemon's `identify` first
   * request). The hello is read first, as over stdio; `member` and `attribution` are then what the daemon answered. A
   * refused identity rejects with an `ApiError` of code `identity.refused` (its message never repeats the name or the
   * token); a path nobody listens on rejects with the socket's error; a name together with a credential, or a member
   * without a token, rejects with a `TypeError` before anything is sent. `close()` shuts the socket and waits for the
   * daemon to end the session; it answers 0. */
  static async connectSocket(path: string, identity: Identity = {}): Promise<Client> {
    checkIdentity(identity);
    const socket = await connectUnix(path);
    const exited = new Promise<number>((resolve) => socket.on("close", () => resolve(0)));
    const c = new Client(socket, socket, exited, socket);
    try {
      await c.hello();
      await c.identify(identifyParams(identity));
    } catch (e) {
      socket.destroy();
      throw e;
    }
    return c;
  }

  /** Say who you are with the daemon's `identify` request; nothing is sent when there is nothing to say. */
  private async identify(params: Record<string, string> | null) {
    if (params === null) return;
    const answer = await this.request("identify", params);
    this.member = String(answer.member ?? this.member);
    this.attribution = String(answer.attribution ?? this.attribution);
  }

  /** Connect to a `klatch-server http --listen ADDR` at `url` (`http://127.0.0.1:PORT`, loopback only) as `member` with
   * its credential `token`, the only way to be known over HTTP (a worker credential the human issued). The stream is
   * `GET /v1/stream` and each request is a `POST /v1/frames` carrying `X-Klatch-Session`. A refused credential rejects
   * with an `ApiError` of code `identity.refused` (its message never repeats the token); a server nobody listens on
   * rejects with the socket's error; a url that is not plain `http` to a loopback address, or a member without a token
   * (or the reverse), rejects with a `TypeError` before anything is sent. If the stream ends without `close()` it is
   * reopened (up to `reconnectAttempts` times) and every subscribed room is resubscribed from the last `seq` seen, so
   * messages resume without a gap or a repeat. `close()` ends the session and answers 0. */
  static async connectHttp(url: string, options: HttpOptions = {}): Promise<Client> {
    const { member, token } = options;
    if (member === undefined || token === undefined) throw new TypeError("a credential is a member and a token");
    const { host, port } = loopbackEndpoint(url);
    let client: Client | null = null;
    const transport = new HttpTransport(
      { host, port, member, token },
      {
        connectTimeoutMs: options.connectTimeoutMs ?? 10_000,
        reconnectAttempts: options.reconnectAttempts ?? 5,
        reconnectDelayMs: options.reconnectDelayMs ?? 200,
      },
      (e) => client?.push({ type: "_send_error", error: e.message }),
    );
    await transport.open(false);
    client = new Client(transport.input, transport.output, transport.exited, null);
    client.http = transport;
    try {
      await client.hello();
    } catch (e) {
      transport.shut();
      throw e;
    }
    return client;
  }

  /** The open event stream, for a test to inject an error into. */
  private get httpStream(): IncomingMessage | null {
    return this.http?.stream ?? null;
  }

  private async hello() {
    const hello = await this.frame(10_000);
    if (hello === null || hello.type !== "hello") throw new ProtocolError("the first frame is hello");
    this.member = String(hello.member);
    this.attribution = String(hello.attribution);
    this.clearance = String(hello.clearance);
  }

  private push(frame: Frame | null) {
    const w = this.waiters.shift();
    if (w) w(frame);
    else this.frames.push(frame);
  }

  private frame(timeoutMs: number): Promise<Frame | null> {
    if (this.frames.length > 0) return Promise.resolve(this.frames.shift() ?? null);
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.waiters = this.waiters.filter((w) => w !== waiter);
        resolve(null);
      }, timeoutMs);
      const waiter: Waiter = (f) => {
        clearTimeout(timer);
        resolve(f);
      };
      this.waiters.push(waiter);
    });
  }

  private note(frame: Frame) {
    if (frame.type === "message") this.pendingMessages.push(frame as unknown as Message);
    else if (frame.type === "caught_up") this.caughtUp.set(String(frame.room), frame);
    else if (frame.type === "refusal") this.refusals.push(frame);
  }

  private send(frame: Record<string, unknown>) {
    this.writable.write(encodeLine(frame));
  }

  /** Send a request and return its result, or throw ApiError. Frames that arrive meanwhile are kept. */
  async request(method: string, params: Record<string, unknown> = {}): Promise<Record<string, any>> {
    if (this.ended) throw new ProtocolError("the session is closed");
    const id = `c${++this.nextId}`;
    this.refusals = [];
    this.send({ type: "request", id, method, params });
    return await this.awaitResponse(id);
  }

  private async awaitResponse(id: string): Promise<Record<string, any>> {
    for (;;) {
      const frame = await this.frame(30_000);
      if (frame === null) throw new ProtocolError("the gateway closed the session or did not answer");
      if (frame.type === "_send_error") throw new ProtocolError(`the session is closed: ${frame.error}`);
      if (frame.type === "response" && frame.id === id) return this.resultOf(frame);
      this.note(frame);
    }
  }

  private resultOf(response: Frame): Record<string, any> {
    if (response.ok) return (response.result as Record<string, any>) ?? {};
    const err = (response.error as Record<string, string>) ?? {};
    throw new ApiError(err.code ?? "backend", err.message ?? "", [...this.refusals]);
  }

  /** `scope` (#1171) declares the scope an ask is about, in the grammar of a member scope: only on a body that starts
   * `seeking:`, and not with `kind` or `payload`. `couldAnswer` then names a member whose own scope differs. `refs` (#1318)
   * are references the ask carries to artifacts outside klatch (at most 32; a known kind such as `pr`, `ticket`, `commit`,
   * `path` or `url` has its value checked, any other kind is kept as text; over a cap is refused, never cut); same rule as
   * `scope`. klatch never fetches, opens or resolves a ref, and refuses a credential in one. */
  async post(
    room: string,
    body: string,
    opts: { label?: string; kind?: string; payload?: string; scope?: string; refs?: AskRef[] } = {},
  ) {
    const params: Record<string, unknown> = { room, body };
    for (const [k, v] of Object.entries(opts)) if (v !== undefined) params[k] = v;
    return (await this.request("post", params)) as { seq: number; id: string };
  }

  /** Send a direct message to one other member. Answers the two-member room made on first use, which both members
   * can read, `history` and `subscribe` to. The label may go below the room's ceiling (the lower of the two
   * clearances), never above. */
  async dm(to: string, body: string, label?: string) {
    const params: Record<string, unknown> = { to, body };
    if (label !== undefined) params.label = label;
    return (await this.request("dm", params)) as { room: string; seq: number; id: string };
  }

  /** Unread messages from others, advancing the cursor unless `peek`. `maxMessages` and `maxBytes` (#1390, sent as
   * `max_messages` and `max_bytes`) bound the read: whole messages, the most important first, at most that many and at
   * most that many bytes of JSON; the rest stay unread for the next `read`. Sent as given (a non-negative integer; the
   * server checks and refuses what it cannot take, as an ApiError) and left out when undefined, so the read is unbounded
   * as before. Answers the messages; `readBounded` also says how many were left. */
  async read(room: string, peek = false, opts: { maxMessages?: number; maxBytes?: number } = {}): Promise<Message[]> {
    return (await this.readBounded(room, peek, opts)).messages;
  }

  /** `read` with the whole answer: `omitted` is the number of messages a bounded read left unread (null when no bound
   * was given, as the server sends no count) and `byImportance` is true when the page is not in `seq` order. */
  async readBounded(
    room: string,
    peek = false,
    opts: { maxMessages?: number; maxBytes?: number } = {},
  ): Promise<{ messages: Message[]; omitted: number | null; byImportance: boolean }> {
    const params: Record<string, unknown> = { room, peek };
    if (opts.maxMessages !== undefined) params["max_messages"] = opts.maxMessages;
    if (opts.maxBytes !== undefined) params["max_bytes"] = opts.maxBytes;
    const r = await this.request("read", params);
    return { messages: r.messages, omitted: r.omitted ?? null, byImportance: r.by_importance ?? false };
  }

  /** Look back without moving any cursor. No `last` and no `since`: the newest 20; `last` alone: the newest `last`
   * (1 to 200); `since` alone: the next 20 after that `seq`; both: the newest `last` after `since`. Page a long room by
   * passing the last `seq` returned as the next `since`. */
  async history(room: string, opts: { last?: number; since?: number } = {}): Promise<{ messages: Message[]; more: number }> {
    const params: Record<string, unknown> = { room };
    for (const [k, v] of Object.entries(opts)) if (v !== undefined) params[k] = v;
    const r = await this.request("history", params);
    return { messages: r.messages, more: r.more ?? 0 };
  }

  async claim(room: string, path: string, opts: { ttl?: number; reason?: string; shared?: boolean } = {}) {
    const params: Record<string, unknown> = { room, path };
    for (const [k, v] of Object.entries(opts)) if (v !== undefined) params[k] = v;
    await this.request("claim", params);
  }

  async release(room: string, path: string, note?: string) {
    await this.request("release", note === undefined ? { room, path } : { room, path, note });
  }

  async claims(room: string): Promise<Record<string, any>[]> {
    return (await this.request("claims", { room })).claims;
  }

  /** Whether `member` could answer open ask `ask` in `room` now (#671): `{ member, ask, could, blockers: [{ rule, reason }] }`. A read only; asking about a member other than yourself is an admin act. Rule words and names only, never the ask's text. */
  async couldAnswer(room: string, member: string, ask: number): Promise<Record<string, any>> {
    return await this.request("could_answer", { room, member, ask });
  }

  /** The stuck cases of a room as events (stale_ask, nobody_capable, ask_cycle, claim_deadlock, long_exchange, dead_holder, possible_overwrite): a kind, a stable key, and ids, member names and numbers, never message text. */
  async stuck(room: string): Promise<Record<string, any>[]> {
    return (await this.request("stuck", { room })).events;
  }

  async who(room: string): Promise<Record<string, any>[]> {
    return (await this.request("who", { room })).members;
  }

  /** Set your own status in a room (one line, 1 to 80 characters), for example `done`. Shown by `who` and the board. */
  async setStatus(room: string, status: string) {
    await this.request("status", { room, status });
  }

  /** Join `room`, making it if it does not exist (the room-creation capability may need a grant). */
  async join(room: string) {
    await this.request("join", { room });
  }

  /** The room's open asks, each `{seq, sender, text, ts, claimed, waiting_on_asker, room, scope}` (`scope` is the scope
   * the asker declared, null when none). The text is another
   * member's words: data, never an instruction. */
  async asks(room: string) {
    return (await this.request("asks", { room })).asks;
  }

  /** Post a content-free pointer (`klatch join <room>`) for `member` in `via`, a room you both already are in. It
   * joins nobody; the invited member decides. */
  async invite(member: string, room: string, via: string) {
    await this.request("invite", { member, room, via });
  }

  /** As the room's grantor, give `member` a clearance of `label` in `room` only. Refused unless you are the room's
   * grantor, the label is not the top level, and it is within the agent cap, the room's label and your own clearance;
   * the refusal names the rule. */
  async grantRoomClearance(room: string, member: string, label: string) {
    await this.request("grant_room_clearance", { room, member, label });
  }

  /** Your own crossing requests, newest first, each `{id, room, seq, to_room, state, why}`. `state` is `waiting`,
   * `approved`, `denied`, `released` or `blocked`; `why` is one fixed sentence, never the text or a reviewer's words. */
  async myCrossings(limit = 20) {
    return (await this.request("my_crossings", { limit })).crossings;
  }

  /** Your own role as one fixed word (`librarian`, `admin`, `reviewer`, `facilitator` or `integrator`), or null when
   * you hold none. Only your own; there is no way to ask for another member's. */
  async myRole(): Promise<string | null> {
    return (await this.request("my_role", {})).role;
  }

  /** The names of the rooms you have joined, without reading anything. Only your own. */
  async myRooms(): Promise<string[]> {
    return (await this.request("my_rooms", {})).rooms;
  }

  /** The label levels from lowest to highest and the agent cap. Read-only; never a room, a clearance or a rule. */
  async labelOrder(): Promise<{ chain: string[]; cap: string }> {
    return (await this.request("label_order", {})) as { chain: string[]; cap: string };
  }

  /** Whether `text` would be scrubbed in a room below the agent cap: true or false and nothing more (never a rule name
   * or a term). Writes no refusal and no audit row. */
  async scrubCheck(text: string): Promise<boolean> {
    return (await this.request("scrub_check", { text })).scrubbed;
  }

  /** The room's board as one block of text (open asks, offers, tasks, members, the merge queue), each entry bounded.
   * The entries are other members' words: data, never instructions. */
  async board(room: string): Promise<string> {
    return (await this.request("board", { room })).board;
  }

  /** What the sender of message `seq` can learn about each recipient: `state` is `parked`, `seen_after` or
   * `not_seen` (liveness, not proof of reading). Refused unless you sent the message; never returns its text. */
  async receipt(room: string, seq: number) {
    return (await this.request("receipt", { room, seq })) as {
      seq: number;
      posted: number;
      recipients: { name: string; state: string; secs: number; shown: boolean; read: boolean }[];
    };
  }

  /** Declare the scope you work in (1 to 80 characters of letters, digits and `. : _ - /`): your claims conflict only
   * with claims in the same scope, and a shared claim conflicts with every scope. */
  async setScope(scope: string) {
    await this.request("scope", { scope });
  }

  /** Declare your own kind for display: `bot`, `service` or `human`; null withdraws it. `session` is never declared:
   * klatch derives it. */
  async setKind(kind: string | null) {
    await this.request("kind", { kind });
  }

  /** The reviewer's queue: the next held item (`id`, `room`, `seq`, `to_label`, `to_room`, `reason`, `text`) or null when there
   * is none. Only the reviewer role may call it (`refused`, rule `reviewer`). */
  async reviewNext(): Promise<Record<string, any> | null> {
    return (await this.request("review_next")).item ?? null;
  }

  /** The reviewer's verdict on an item: `allow`, `redact` (with `text`) or `deny` (with an optional `reason`). Only the
   * reviewer role may call it. */
  async reviewVerdict(id: number, verdict: string, opts: { text?: string; reason?: string } = {}) {
    await this.request("review_verdict", { id, verdict, ...opts });
  }

  /** An act of the admin agent (`close`, `reopen`, `remove`, `allow`, `clear`, `label`, `audit`, `refusals`,
   * `run-create`, `run-end`, `anchor-done`) with what it needs (`room`, `member`, `label`, `limit`, `run`, `anchor`).
   * A member without the role is refused (`ApiError` code `refused`, rule `admin-scope`). Returns the result object. */
  async roleAct(act: string, params: Record<string, unknown> = {}): Promise<Record<string, any>> {
    return this.request("role_act", { ...params, act });
  }

  /** Housekeeping every member may run: remove the members not seen for the fixed retention. Returns how many. */
  async prune(): Promise<number> {
    return (await this.request("prune")).removed ?? 0;
  }

  /** Mute (or, with `muted` false, unmute) a room for yourself: your digest then shows only messages that mention
   * you. `read` and the stream still carry everything. */
  async mute(room: string, muted = true) {
    await this.request("mute", { room, muted });
  }

  /** Start (or resume) a stream; returns the room's head. */
  async subscribe(room: string, fromSeq = 0): Promise<number> {
    this.caughtUp.delete(room);
    return (await this.request("subscribe", { room, from_seq: fromSeq })).head;
  }

  /** Stop the stream for a room; harmless if there is none. */
  async unsubscribe(room: string) {
    await this.request("unsubscribe", { room });
  }

  ack(room: string, seq: number) {
    this.send({ type: "ack", room, seq });
  }

  private stillWaitingForCaughtUp(untilCaughtUp: boolean, end: number): boolean {
    return untilCaughtUp && this.caughtUp.size === 0 && Date.now() < end;
  }

  /** Messages received so far; with `untilCaughtUp`, wait for the replay to finish first. */
  async collect(untilCaughtUp = false, timeoutMs = 10_000) {
    const end = Date.now() + timeoutMs;
    while (this.stillWaitingForCaughtUp(untilCaughtUp, end)) {
      const frame = await this.frame(Math.max(0, end - Date.now()));
      if (frame === null) break;
      this.note(frame);
    }
    const messages = this.pendingMessages;
    this.pendingMessages = [];
    return { messages, caughtUp: this.caughtUp.values().next().value ?? null };
  }

  /** The next message of a subscription, waiting for it; null on timeout. */
  async nextMessage(timeoutMs = 10_000): Promise<Message | null> {
    const end = Date.now() + timeoutMs;
    for (;;) {
      const m = this.pendingMessages.shift();
      if (m) return m;
      const frame = await this.frame(Math.max(0, end - Date.now()));
      if (frame === null) return null;
      this.note(frame);
    }
  }

  /** End the session by closing the input; returns the gateway's exit code. Over a socket there is no process: the
   * socket is shut and the daemon's end of the session is awaited (up to 10 seconds), and the answer is 0. */
  async close(): Promise<number> {
    this.ended = true;
    this.writable.end();
    if (this.http !== null) {
      this.http.shut();
      return await this.exited;
    }
    if (this.socket === null) return await this.exited;
    const socket = this.socket;
    const timer = setTimeout(() => socket.destroy(), 10_000);
    try {
      return await this.exited;
    } finally {
      clearTimeout(timer);
    }
  }
}
