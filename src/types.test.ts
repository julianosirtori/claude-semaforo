import { describe, expect, it } from "vitest";
import { derive, nextCue, relTime, type Session, type SessionState } from "./types";

function session(id: string, state: Session["state"]): Session {
  return { id, folder: id, cwd: `/x/${id}`, container: false, state, lastMsg: "", updatedAt: 0 };
}

function prev(...pairs: [string, SessionState][]): Map<string, SessionState> {
  return new Map(pairs);
}

describe("derive", () => {
  it("is empty/green with no sessions", () => {
    const d = derive([]);
    expect(d.total).toBe(0);
    expect(d.worst).toBe("ready");
    expect(d.badgeCount).toBe(0);
    expect(d.hasWait).toBe(false);
    expect(d.headerSub).toBe("0 sessões · 0 prontas");
  });

  it("picks waiting as the worst state and counts it", () => {
    const d = derive([
      session("a", "waiting"),
      session("b", "waiting"),
      session("c", "working"),
      session("d", "ready"),
      session("e", "ready"),
    ]);
    expect(d.worst).toBe("waiting");
    expect(d.badgeCount).toBe(2);
    expect(d.hasWait).toBe(true);
    expect(d.counts).toEqual({ waiting: 2, working: 1, ready: 2 });
    expect(d.headerSub).toBe("5 sessões · 2 esperando");
  });

  it("falls back through working before ready", () => {
    expect(derive([session("a", "working"), session("b", "ready")]).worst).toBe("working");
    expect(derive([session("a", "ready"), session("b", "ready")]).worst).toBe("ready");
  });

  it("uses the singular for a lone session", () => {
    expect(derive([session("a", "working")]).headerSub).toBe("1 sessão · 1 trabalhando");
  });
});

describe("nextCue", () => {
  it("cues a session appearing already in waiting (the bug fix)", () => {
    // First event is a Notification: no prior state, still must chime.
    expect(nextCue(prev(), [session("a", "waiting")])).toBe("waiting");
  });

  it("does not cue a session appearing in ready", () => {
    expect(nextCue(prev(), [session("a", "ready")])).toBe(null);
  });

  it("cues on transitions into waiting and ready", () => {
    expect(nextCue(prev(["a", "working"]), [session("a", "waiting")])).toBe("waiting");
    expect(nextCue(prev(["a", "working"]), [session("a", "ready")])).toBe("ready");
  });

  it("stays quiet when nothing changed", () => {
    expect(nextCue(prev(["a", "waiting"]), [session("a", "waiting")])).toBe(null);
    expect(nextCue(prev(["a", "ready"]), [session("a", "ready")])).toBe(null);
  });

  it("lets waiting win over a simultaneous ready", () => {
    const cue = nextCue(prev(["a", "working"], ["b", "working"]), [
      session("a", "ready"),
      session("b", "waiting"),
    ]);
    expect(cue).toBe("waiting");
  });
});

describe("relTime", () => {
  const now = 1_000_000_000_000;
  it("clips recent timestamps to 'agora'", () => {
    expect(relTime(now, now)).toBe("agora");
    expect(relTime(now - 3_000, now)).toBe("agora");
  });
  it("formats seconds, minutes and hours", () => {
    expect(relTime(now - 10_000, now)).toBe("10 s");
    expect(relTime(now - 120_000, now)).toBe("2 min");
    expect(relTime(now - 2 * 3_600_000, now)).toBe("2 h");
  });
});
