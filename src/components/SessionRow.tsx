import { relTime, STATE_CLASS, STATE_LABEL, type Session } from "../types";

interface Props {
  session: Session;
  flash: boolean;
  nowMs: number;
  onAllow: () => void;
  onAlways: () => void;
  onDeny: () => void;
}

export function SessionRow({ session: s, flash, nowMs, onAllow, onAlways, onDeny }: Props) {
  const isPerm = s.state === "waiting" && s.reqKind === "perm";
  const isAsk = s.state === "waiting" && s.reqKind === "ask";

  return (
    <div className={`row ${STATE_CLASS[s.state]}${flash ? " row--flash" : ""}`}>
      <div className="row__main">
        <span className="row__dot" />
        <div className="row__body">
          <div className="row__line">
            <span className="row__folder">{s.folder}</span>
            {s.container && <span className="tag">container</span>}
            <span className="statepill">{STATE_LABEL[s.state]}</span>
          </div>
          <div className="row__path">{s.cwd} · {relTime(s.updatedAt, nowMs)}</div>
          <div className="row__msg">{s.lastMsg}</div>

          {isPerm && (
            <>
              <div className="code">
                <span className="code__p">$</span>
                <span className="code__cmd">{s.cmd}</span>
              </div>
              <div className="acts">
                <button className="btn btn--allow" onClick={onAllow}>Permitir</button>
                <button className="btn btn--ghost" onClick={onAlways}>Sempre</button>
                <button className="btn btn--deny" onClick={onDeny}>Negar</button>
              </div>
            </>
          )}

          {isAsk && (
            // A generic "ask" (from a Notification) has no channel back to the
            // pill, so point the user at the terminal.
            <div className="reply-terminal">responda no seu terminal ↵</div>
          )}
        </div>
      </div>
    </div>
  );
}
