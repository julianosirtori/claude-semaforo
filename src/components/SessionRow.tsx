import { relTime, STATE_CLASS, STATE_LABEL, type Session } from "../types";

interface Props {
  session: Session;
  flash: boolean;
  nowMs: number;
}

export function SessionRow({ session: s, flash, nowMs }: Props) {
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
        </div>
      </div>
    </div>
  );
}
