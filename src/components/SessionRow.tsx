import { relTime, STATE_CLASS, STATE_LABEL, type Session } from "../types";

interface Props {
  session: Session;
  flash: boolean;
  nowMs: number;
  draft: string;
  onDraft: (v: string) => void;
  onAllow: () => void;
  onAlways: () => void;
  onDeny: () => void;
  onSend: () => void;
}

export function SessionRow({ session: s, flash, nowMs, draft, onDraft, onAllow, onAlways, onDeny, onSend }: Props) {
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
            <>
              <div className="reply">
                <input
                  className="reply__in"
                  value={draft}
                  onChange={(e) => onDraft(e.target.value)}
                  onKeyDown={(e) => { if (e.key === "Enter") onSend(); }}
                  placeholder="Responder o Claude…"
                />
                <button className="btn btn--send" onClick={onSend}>Enviar</button>
              </div>
              <div className="note">resposta em texto roda no modo SDK</div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
