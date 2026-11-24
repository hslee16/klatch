"""The watch loop's two reads (split out of runexec, #1519): the stream pump and the hidden-checker poll.

pump stamps every stream line with its own clock reading (a batch of lines read in one go never shares one time, which
asks._is_late and the answer delays rely on); poll_checker runs the hidden checker in a thread, so a slow checkout or
checker (seconds, on a loaded machine) never holds the loop up and makes lines arrive in batches."""
import threading
import time


class PollSupport:
    def pump(self):
        for name in self.procs:
            with open(self.log_path(name, "stream"), "rb") as f:
                f.seek(self.offs[name])
                chunk = f.read()
            self.offs[name] += len(chunk)
            data = self.partial[name] + chunk
            *lines, self.partial[name] = data.split(b"\n")
            for ln in lines:
                now = round(time.monotonic() - self.t0, 6)
                self.states[name].feed(ln.decode("utf-8", "replace"), now)
                self.stamps[name].append(now)

    def poll_checker(self):
        running = getattr(self, "poll_thread", None)
        if running is not None and running.is_alive():
            return
        if self.first_pass is None and time.monotonic() - self.last_poll >= self.a.poll_s:
            self.last_poll = time.monotonic()
            self.poll_thread = threading.Thread(target=self.poll_once, daemon=True)
            self.poll_thread.start()

    def poll_once(self):
        checks, _, err = self.checks_now("poll")
        passed = err is None and self.all_pass(checks)
        if passed and self.first_pass is None:
            self.first_pass = time.monotonic() - self.t0

    def join_poll(self):
        running = getattr(self, "poll_thread", None)
        if running is not None:
            running.join()
