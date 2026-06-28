import { describe, expect, it } from "vitest";
import { derive, relTime, type Session } from "./types";

function session(id: string, state: Session["state"]): Session {
  return { id, folder: id, cwd: `/x/${id}`, container: false, state, reqKind: null, cmd: null, lastMsg: "", updatedAt: 0 };
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
