import { Glyph } from "./Glyph";
import { ChevronLeft, Close, Gear } from "./icons";
import { SessionRow } from "./SessionRow";
import { ConfigView } from "./ConfigView";
import { STATE_CLASS, type AppConfig, type Derived, type Snapshot, type SessionState } from "../types";

export const APP_VERSION = "0.1.0";
const COUNT_ORDER: SessionState[] = ["waiting", "working", "ready"];
const COUNT_LABEL: Record<SessionState, string> = { waiting: "esperando", working: "trabalhando", ready: "prontas" };

interface Props {
  open: boolean;
  view: "list" | "config";
  snapshot: Snapshot;
  derived: Derived;
  flashId: string | null;
  nowMs: number;
  regenSpinning: boolean;
  onShowConfig: () => void;
  onShowList: () => void;
  onClose: () => void;
  onPatch: (patch: Partial<AppConfig>) => void;
  onCopyToken: () => void;
  onRegenToken: () => void;
  hooksInstalled: boolean;
  installing: boolean;
  onInstallHooks: () => void;
  onCopyContainer: () => void;
  onQuit: () => void;
  onAllow: (id: string) => void;
  onAlways: (id: string) => void;
  onDeny: (id: string) => void;
}

export function Panel(p: Props) {
  const { snapshot, derived, view } = p;
  const cfg: AppConfig = snapshot.config;
  const port = String(cfg.bind).split(":").pop() ?? "7337";
  const isList = view === "list";
  const empty = derived.total === 0;

  return (
    <div className={`panel ${p.open ? "panel--open" : "panel--closed"}`} onClick={(e) => e.stopPropagation()}>
      <div className="card">
        {/* Header */}
        <div className="hd">
          {isList
            ? <Glyph box={30} dot={6} />
            : <button className="iconbtn iconbtn--back" title="Voltar" onClick={p.onShowList}><ChevronLeft /></button>}
          <div className="hd__grow">
            {isList ? (
              <>
                <div className="hd__title">Claude Semáforo</div>
                <div className="hd__sub">{derived.headerSub}</div>
              </>
            ) : (
              <>
                <div className="hd__title">Configuração</div>
                <div className="hd__sub hd__sub--mono">v{APP_VERSION} · porta {port}</div>
              </>
            )}
          </div>
          {isList && <button className="iconbtn iconbtn--gear" title="Configuração" onClick={p.onShowConfig}><Gear /></button>}
          <button className="iconbtn" title="Fechar" onClick={p.onClose}><Close /></button>
        </div>

        {/* Body */}
        {isList ? (
          empty ? (
            <>
              <div className="empty">
                <div className="empty__t">Nenhuma sessão conectada</div>
                <div className="empty__h">cp hooks/notify.sh ~/.claude/</div>
              </div>
              <div className="ft">
                <span className="ft__dot ft__dot--idle" />
                <span className="ft__txt">escutando · aguardando</span>
              </div>
            </>
          ) : (
            <>
              <div className="counts">
                {COUNT_ORDER.map((st) => (
                  <span key={st} className={`count ${STATE_CLASS[st]}`}>
                    <span className="count__dot" />
                    <span className="count__n">{derived.counts[st]}</span>
                    <span className="count__w">{COUNT_LABEL[st]}</span>
                  </span>
                ))}
              </div>
              <div className="rows">
                {[...snapshot.sessions]
                  .sort((a, b) => COUNT_ORDER.indexOf(a.state) - COUNT_ORDER.indexOf(b.state) || b.updatedAt - a.updatedAt)
                  .map((s) => (
                    <SessionRow
                      key={s.id}
                      session={s}
                      flash={p.flashId === s.id}
                      nowMs={p.nowMs}
                      onAllow={() => p.onAllow(s.id)}
                      onAlways={() => p.onAlways(s.id)}
                      onDeny={() => p.onDeny(s.id)}
                    />
                  ))}
              </div>
              <div className="ft">
                <span className="ft__dot" />
                <span className="ft__txt">escutando · {cfg.bind}</span>
                <span className="ft__right">{String(cfg.bind).startsWith("0.0.0.0") ? "porta aberta" : "só host"}</span>
              </div>
            </>
          )
        ) : (
          <ConfigView
            config={cfg}
            onPatch={p.onPatch}
            onCopyToken={p.onCopyToken}
            onRegenToken={p.onRegenToken}
            regenSpinning={p.regenSpinning}
            hooksInstalled={p.hooksInstalled}
            installing={p.installing}
            onInstallHooks={p.onInstallHooks}
            onCopyContainer={p.onCopyContainer}
            onQuit={p.onQuit}
          />
        )}
      </div>
    </div>
  );
}
