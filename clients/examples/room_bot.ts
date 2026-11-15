// The smallest klatch integration: read a room's history, post, resume. Node's standard library only.
//
//   KLATCH_NAME=my-bot node --experimental-strip-types room_bot.ts ROOM
//   (or KLATCH_IDENTITY_FILE=/path/to/bundle for a verified member)
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";

const KLATCH = process.env.KLATCH_BIN ?? "klatch";
type Frame = { type: string; [key: string]: any };

// Spawn klatch and hand each frame (one JSON object per line) to `onFrame`; return false from it to stop reading.
function run(args: string[], onFrame: (f: Frame, send: (f: object) => void) => boolean | void): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = spawn(KLATCH, args, { stdio: ["pipe", "pipe", "inherit"] });
    const send = (f: object) => child.stdin.write(JSON.stringify({ v: 1, ...f }) + "\n");
    createInterface({ input: child.stdout }).on("line", (line) => {
      if (!line.trim()) return;
      const frame: Frame = JSON.parse(line);
      if (frame.v !== 1) return reject(new Error(`unsupported protocol version: ${frame.v}`));
      // a frame type you do not know is yours to ignore
      if (onFrame(frame, send) === false) child.stdin.end(); // closing the input ends the session
    });
    child.on("close", (code) => (code === 0 ? resolve() : reject(new Error(`klatch ${args[0]} failed`))));
  });
}

// Hello, then every message after fromSeq, then caught_up. `klatch stream` needs no daemon.
async function history(room: string, fromSeq = 0) {
  let who = "";
  const messages: Frame[] = [];
  await run(["stream", room, "--from-seq", String(fromSeq)], (f) => {
    if (f.type === "hello") who = `${f.member} (${f.attribution})`;
    else if (f.type === "message") messages.push(f);
    else if (f.type === "caught_up") return false;
  });
  return { who, messages };
}

// One request on a `serve --stdio` session; the response carrying our id is the answer.
async function post(room: string, body: string): Promise<number> {
  let seq = -1;
  await run(["serve", "--stdio"], (f, send) => {
    if (f.type === "hello") send({ type: "request", id: "1", method: "post", params: { room, body } });
    else if (f.type === "response" && f.id === "1") {
      if (!f.ok) throw new Error(`refused: ${f.error.code}: ${f.error.message}`);
      seq = f.result.seq;
      return false;
    }
  });
  return seq;
}

const room = process.argv[2];
const { who, messages } = await history(room);
console.log("member:", who);
for (const m of messages) console.log(`history: ${m.sender}: ${m.body}`);
const lastSeq = messages.length ? messages[messages.length - 1].seq : 0;
const me = who.split(" ")[0];
console.log("posted: seq", await post(room, `hello from ${me}`));
const later = await history(room, lastSeq); // resume: only what came after the last seq we processed
for (const m of later.messages) console.log(`resumed: ${m.sender}: ${m.body}`);
