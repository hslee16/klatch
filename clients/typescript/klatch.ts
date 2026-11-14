// A thin klatch client over the wire protocol (PROTOCOL.md, version 1): newline-delimited JSON to
// `klatch serve --stdio`, using only Node's standard library. Who you are is decided by the environment of the
// process, never by a request parameter. A frame type or field this version does not know is ignored.

import { spawn, type ChildProcessByStdio } from "node:child_process";
import { createInterface } from "node:readline";
import type { Readable, Writable } from "node:stream";

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
  [key: string]: unknown;
};

/** One line from the gateway as a frame, or null for a blank line. An unknown type or field is not an error. */
export function decodeServerFrame(line: string): Frame | null {
  if (line.trim() === "") return null;
  let frame: unknown;
  try {
    frame = JSON.parse(line);
  } catch (e) {
    throw new ProtocolError(`not JSON: ${(e as Error).message}`);
  }
  if (typeof frame !== "object" || frame === null || Array.isArray(frame)) {
    throw new ProtocolError("a frame is a JSON object");
  }
  const f = frame as Record<string, unknown>;
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

/** One session with the gateway. */
export class Client {
  member = "";
  attribution = "";
  clearance = "";
  refusals: Frame[] = [];
  private proc: ChildProcessByStdio<Writable, Readable, null>;
  private frames: (Frame | null)[] = [];
  private waiters: Waiter[] = [];
  private nextId = 0;
  private pendingMessages: Message[] = [];
  private caughtUp = new Map<string, Frame>();
  private exited: Promise<number>;

  private constructor(binary: string, env: NodeJS.ProcessEnv, args: string[]) {
    this.proc = spawn(binary, args, { env, stdio: ["pipe", "pipe", "ignore"] });
    createInterface({ input: this.proc.stdout }).on("line", (line) => {
      let frame: Frame | null;
      try {
        frame = decodeServerFrame(line);
      } catch (e) {
        frame = { type: "_error", error: String(e) };
      }
      if (frame !== null) this.push(frame);
    });
    this.proc.stdout.on("close", () => this.push(null));
    this.exited = new Promise((resolve) => this.proc.on("close", (code) => resolve(code ?? -1)));
  }

  static async connect(binary: string, env: NodeJS.ProcessEnv, args: string[] = ["serve", "--stdio"]): Promise<Client> {
    const c = new Client(binary, env, args);
    const hello = await c.frame(10_000);
    if (hello === null || hello.type !== "hello") throw new ProtocolError("the first frame is hello");
    c.member = String(hello.member);
    c.attribution = String(hello.attribution);
    c.clearance = String(hello.clearance);
    return c;
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
    this.proc.stdin.write(encodeLine(frame));
  }

  /** Send a request and return its result, or throw ApiError. Frames that arrive meanwhile are kept. */
  async request(method: string, params: Record<string, unknown> = {}): Promise<Record<string, any>> {
    const id = `c${++this.nextId}`;
    this.refusals = [];
    this.send({ type: "request", id, method, params });
    for (;;) {
      const frame = await this.frame(30_000);
      if (frame === null) throw new ProtocolError("the gateway closed the session or did not answer");
      if (frame.type === "response" && frame.id === id) {
        if (frame.ok) return (frame.result as Record<string, any>) ?? {};
        const err = (frame.error as Record<string, string>) ?? {};
        throw new ApiError(err.code ?? "backend", err.message ?? "", [...this.refusals]);
      }
      this.note(frame);
    }
  }

  async post(room: string, body: string, opts: { label?: string; kind?: string; payload?: string } = {}) {
    const params: Record<string, unknown> = { room, body };
    for (const [k, v] of Object.entries(opts)) if (v !== undefined) params[k] = v;
    return (await this.request("post", params)) as { seq: number; id: string };
  }

  async read(room: string, peek = false): Promise<Message[]> {
    return (await this.request("read", { room, peek })).messages;
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

  async who(room: string): Promise<Record<string, any>[]> {
    return (await this.request("who", { room })).members;
  }

  /** Start (or resume) a stream; returns the room's head. */
  async subscribe(room: string, fromSeq = 0): Promise<number> {
    this.caughtUp.delete(room);
    return (await this.request("subscribe", { room, from_seq: fromSeq })).head;
  }

  ack(room: string, seq: number) {
    this.send({ type: "ack", room, seq });
  }

  /** Messages received so far; with `untilCaughtUp`, wait for the replay to finish first. */
  async collect(untilCaughtUp = false, timeoutMs = 10_000) {
    const end = Date.now() + timeoutMs;
    while (untilCaughtUp && this.caughtUp.size === 0 && Date.now() < end) {
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

  /** End the session by closing the input; returns the gateway's exit code. */
  async close(): Promise<number> {
    this.proc.stdin.end();
    return await this.exited;
  }
}
