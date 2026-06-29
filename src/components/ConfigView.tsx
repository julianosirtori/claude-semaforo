import { Copy, Refresh } from "./icons";
import { Toggle } from "./Toggle";
import { Segmented } from "./Segmented";
import type { AppConfig, BindAddr, ThemePref } from "../types";

interface Props {
  config: AppConfig;
  onPatch: (patch: Partial<AppConfig>) => void;
  onCopyToken: () => void;
  onRegenToken: () => void;
  regenSpinning: boolean;
  hooksInstalled: boolean;
  installing: boolean;
  onInstallHooks: () => void;
  onCopyContainer: () => void;
  onQuit: () => void;
}

export function ConfigView({ config, onPatch, onCopyToken, onRegenToken, regenSpinning, hooksInstalled, installing, onInstallHooks, onCopyContainer, onQuit }: Props) {
  return (
    <>
      <div className="cfg">
        {/* Conexão */}
        <div>
          <div className="cfg__h">Conexão</div>
          <div className="cfg__group">
            <div>
              <div className="cfg__label">Token (Bearer)</div>
              <div className="token">
                <div className="token__box">••••••••••••••••</div>
                <button className="token__btn" title="Copiar token" onClick={onCopyToken}><Copy /></button>
                <button className="token__btn" title="Regenerar token" onClick={onRegenToken}>
                  <Refresh className={regenSpinning ? "spin" : undefined} />
                </button>
              </div>
            </div>
            <div>
              <div className="cfg__label">Endereço de escuta</div>
              <Segmented<BindAddr>
                value={config.bind}
                onChange={(v) => onPatch({ bind: v })}
                options={[
                  { value: "0.0.0.0:7337", label: "0.0.0.0:7337" },
                  { value: "127.0.0.1:7337", label: "127.0.0.1:7337" },
                ]}
              />
              <div className="help">0.0.0.0 alcança containers. 127.0.0.1 tranca tudo no host.</div>
            </div>
          </div>
        </div>

        <div className="cfg__divider" />

        {/* Claude Code */}
        <div>
          <div className="cfg__h">Claude Code</div>
          <div className="cfg__group">
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Hooks {hooksInstalled ? "instalados" : "não instalados"}</div>
                <div className="cfgrow__s">escreve notify em ~/.claude e registra os eventos</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <button
                className="cfgbtn"
                onClick={(e) => { e.stopPropagation(); onInstallHooks(); }}
                disabled={installing}
              >
                {installing ? "Instalando…" : hooksInstalled ? "Reinstalar" : "Instalar"}
              </button>
            </div>
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Devcontainer</div>
                <div className="cfgrow__s">notify.sh + token + hooks num só script</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <button className="cfgbtn" onClick={(e) => { e.stopPropagation(); onCopyContainer(); }}>Copiar</button>
            </div>
          </div>
        </div>

        <div className="cfg__divider" />

        {/* Respostas */}
        <div>
          <div className="cfg__h">Respostas</div>
          <div className="cfg__group">
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Permitir/negar pela pílula</div>
                <div className="cfgrow__s">via hook HTTP (nativo)</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.replyPerm} onChange={(v) => onPatch({ replyPerm: v })} />
            </div>
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Responder em texto</div>
                <div className="cfgrow__s">requer sessões em modo SDK</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.replyText} onChange={(v) => onPatch({ replyText: v })} />
            </div>
          </div>
        </div>

        <div className="cfg__divider" />

        {/* Aparência & sistema */}
        <div>
          <div className="cfg__h">Aparência &amp; sistema</div>
          <div className="cfg__group">
            <div className="cfgrow">
              <span className="cfgrow__t">Tema</span>
              <span style={{ marginLeft: "auto" }} />
              <Segmented<ThemePref>
                value={config.theme}
                variant="tema"
                onChange={(v) => onPatch({ theme: v })}
                options={[
                  { value: "auto", label: "Auto" },
                  { value: "light", label: "Claro" },
                  { value: "dark", label: "Escuro" },
                ]}
              />
            </div>
            <div className="cfgrow">
              <span className="cfgrow__t">Sempre no topo</span>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.alwaysOnTop} onChange={(v) => onPatch({ alwaysOnTop: v })} />
            </div>
            <div className="cfgrow">
              <span className="cfgrow__t">Iniciar com o sistema</span>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.autostart} onChange={(v) => onPatch({ autostart: v })} />
            </div>
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Notificar quando 🔴</div>
                <div className="cfgrow__s">aviso do sistema ao te esperar</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.notify} onChange={(v) => onPatch({ notify: v })} />
            </div>
            <div className="cfgrow">
              <div style={{ minWidth: 0 }}>
                <div className="cfgrow__t">Som ao mudar de estado</div>
                <div className="cfgrow__s">tom quando te espera ou fica pronto</div>
              </div>
              <span style={{ marginLeft: "auto" }} />
              <Toggle on={config.sound} onChange={(v) => onPatch({ sound: v })} />
            </div>
          </div>
        </div>

        <button className="cfgbtn cfgbtn--quit" onClick={(e) => { e.stopPropagation(); onQuit(); }}>
          Sair do Claude Semáforo
        </button>
      </div>

      <div className="cfg-ft">
        <span className="cfg-ft__repo">github.com/julianosirtori/claude-semaforo</span>
        <a className="cfg-ft__docs" href="https://github.com/julianosirtori/claude-semaforo" target="_blank" rel="noreferrer">Docs →</a>
      </div>
    </>
  );
}
