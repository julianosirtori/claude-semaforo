# Arquitetura — Claude Semáforo

Documento de arquitetura do que foi implementado. A ideia é que você consiga
entender cada peça, como elas conversam, e por que cada decisão foi tomada.

## Visão geral

O Claude Semáforo é um widget *always-on-top* feito em **Tauri 2**: o backend é
Rust rodando no host, e o frontend é React + TypeScript renderizado numa janela
transparente sem decoração. Ele agrega o estado de várias sessões do Claude Code
ao mesmo tempo, inclusive as que rodam dentro de containers, e mostra num relance
o pior estado entre todas. Clicando, abre o painel com a lista por projeto, e nas
sessões 🔴 dá pra responder a permissão ali mesmo.

```
┌─────────────────────────── host ───────────────────────────┐
│                                                             │
│   sessão Claude Code ──hook(notify.sh)──▶ POST /events ─────┼─▶ estado em memória
│                       └hook(PreToolUse)─▶ POST /permission ─┼─▶ segura a request
│                                              ▲              │      (long-poll)
│   ┌──── janela Tauri (transparente) ────┐   │              │
│   │  pílula  ◀──── eventos "snapshot" ───┼───┘              │
│   │  painel  ───── invoke(respond) ──────┼──▶ devolve a decisão pela request
│   └──────────────────────────────────────┘                 │
│                                                             │
│   container ──notify.sh via host.docker.internal:7337──────▶ (mesma porta 0.0.0.0)
└─────────────────────────────────────────────────────────────┘
```

O backend escuta em `0.0.0.0:7337` de propósito, pra ser alcançável de dentro de
containers. Todo request exige `Authorization: Bearer <token>`.

## Os três estados

Cada sessão tem um de três estados, e o badge mostra o **pior** entre todas:

| Estado    | Cor          | Rótulo          | Quando                                   |
|-----------|--------------|-----------------|------------------------------------------|
| `waiting` | 🔴 `#E5484D` | Te esperando    | parou pra perguntar ou pedir permissão   |
| `working` | 🟡 `#E89B1C` | Trabalhando     | pensando, entre o prompt e o fim do turno|
| `ready`   | 🟢 `#2FA968` | Pronto          | terminou o turno, tem output pra revisar |

Ordem de prioridade: `waiting > working > ready`. O contador na pílula é a
quantidade de sessões nesse pior estado. A derivação fica em `derive()` em
`src/types.ts`, e é a única fonte da verdade pro badge, os chips e o subtítulo.

## Frontend (`src/`)

React puro, sem framework de UI. CSS com variáveis pra tema, sem Tailwind.

| Arquivo                  | Responsabilidade                                                       |
|--------------------------|------------------------------------------------------------------------|
| `main.tsx`               | ponto de entrada, monta o `App` e importa o CSS                        |
| `App.tsx`                | estado raiz, assinatura do backend, tema, animação de flash, handlers  |
| `api.ts`                 | ponte com o backend (Tauri) **e** um mock de browser fiel ao protótipo |
| `types.ts`               | modelo de domínio + `derive()` (pior estado, contadores, subtítulo)    |
| `window.ts`              | coreografia da janela: expandir/encolher, ancorar, arrastar            |
| `styles.css`             | design tokens (claro/escuro), classes dos componentes, keyframes       |
| `components/Pill.tsx`    | a pílula compacta (badge + halo pulsante)                              |
| `components/Panel.tsx`   | painel: cabeçalho, chips de contagem, lista de sessões, rodapé         |
| `components/SessionRow.tsx` | a linha de uma sessão, com as ações inline de permissão/resposta    |
| `components/ConfigView.tsx` | a tela de configuração                                              |
| `components/Glyph.tsx`, `Toggle.tsx`, `Segmented.tsx`, `icons.tsx` | peças reutilizáveis        |

### Fluxo de dados

`App` chama `api.subscribe(cb)`. Em Tauri, isso escuta o evento `snapshot` emitido
pelo Rust e também busca o estado inicial via `invoke("get_state")`. Cada
`Snapshot` é `{ sessions, config }`. O `App` guarda o último snapshot, deriva o
resto na hora do render, e dispara ações com `api.respond / setConfig /
regenerateToken / saveWindow`. O `token` nunca viaja no snapshot (a UI busca sob
demanda via `reveal_token`), pra não aparecer em todo evento emitido à webview.

O **flash** (aquele realce sutil na linha que mudou) é calculado no frontend:
quando chega um snapshot novo, o `App` compara o `updatedAt` de cada sessão com o
anterior e realça por 780ms a que mudou. Isso vale tanto pro Tauri quanto pro
mock, sem o backend precisar sinalizar nada.

O **som** segue o mesmo caminho: no mesmo diff de snapshot, o `App` compara o
estado anterior de cada sessão e, quando alguma vira 🔴 waiting ou 🟢 ready (com
`config.sound` ligado), toca um tom sintetizado via Web Audio (`sound.ts`, sem
arquivos de áudio). Waiting tem prioridade quando os dois acontecem no mesmo
snapshot. É distinto do plugin `notification` do backend, que só dispara aviso do
SO quando vira 🔴.

### Tema

`styles.css` define a paleta inteira em variáveis no `:root` (claro) e
`[data-theme="dark"]` (escuro, superfície *near-black* quente). As cores do
semáforo e o acento são variáveis também, então trocar de tema ou de acento é só
trocar variável. O `App` resolve o tema efetivo (Auto segue o
`prefers-color-scheme` do SO) e seta `document.documentElement.dataset.theme` e
`--accent`.

### Mock de browser

`api.ts` detecta Tauri por `"__TAURI_INTERNALS__" in window`. Fora do Tauri (no
`vite dev`), ele usa um mock em memória que reproduz o protótipo de design: as
mesmas sessões, o mesmo *auto-driver* que cicla os estados a cada 2.4s, e as
mesmas ações. Isso deixa a interface desenvolvível no browser sem o shell
desktop. O parâmetro `?static` congela o estado inicial pra inspeção.

### Coreografia da janela (`window.ts`)

A janela é pequena e transparente. Para não virar um capturador de cliques
gigante, ela tem dois tamanhos e redimensiona ao abrir/fechar:

- **Encolhida** (96×96): só a pílula, ancorada no canto inferior direito.
- **Aberta** (398×600): o painel acima da pílula.

Ao alternar, `applyOpen()` recalcula o tamanho e reposiciona ancorando o canto
inferior direito, então a pílula não "pula" de lugar. `beginDrag()` implementa o
arrasto manual pela pílula: um clique sem movimento conta como toque (abre/fecha),
qualquer movimento arrasta a janela e persiste o canto via `save_window`. Tudo
isso vira no-op fora do Tauri.

## Backend (`src-tauri/src/`)

| Arquivo        | Responsabilidade                                                        |
|----------------|-------------------------------------------------------------------------|
| `main.rs`      | só chama `run()`                                                         |
| `lib.rs`       | `run()`: plugins, `setup` (carrega config, sobe o server, posiciona a janela, autostart, sweeper), registra os comandos |
| `state.rs`     | modelo de domínio, `Config`, `Snapshot`, `Decision`, `Inner`, `AppState`|
| `config.rs`    | persistência da config em JSON e geração de token                       |
| `server.rs`    | listener HTTP, autenticação, `/events`, `/permission`                   |
| `commands.rs`  | os comandos expostos ao frontend                                        |
| `setup.rs`     | instalação 1-clique dos hooks no `~/.claude` (escreve scripts + mescla o settings.json) |

### Modelo de estado

Tudo vive em memória, atrás de um `Arc<Mutex<Inner>>`:

```rust
struct Inner {
    sessions: HashMap<String, Session>,      // id da sessão -> sessão
    config: Config,
    pending: HashMap<String, Sender<Decision>>, // permissões seguradas, por sessão
    allow_rules: HashSet<String>,            // comandos que você escolheu "Sempre"
}
```

O mesmo `Arc<Mutex<Inner>>` é compartilhado entre os comandos do Tauri e a thread
do servidor HTTP. O `AppState` gerenciado pelo Tauri guarda esse `Arc` mais o
controle do servidor (pra reiniciar quando o bind muda).

### Servidor HTTP (`server.rs`)

Usei `tiny_http` (síncrono, poucas dependências) em vez de axum, pra ficar
enxuto. Uma thread roda o loop de `accept` com `recv_timeout`, e cada request vai
pra uma thread própria. Isso importa por causa do long-poll: uma request de
permissão fica bloqueada esperando você decidir, sem travar as outras.

Autenticação: todo request precisa de `Authorization: Bearer <token>`. A
comparação é em tempo constante (`constant_time_eq`) pra não vazar o token por
timing. Sem token válido, `401`.

**`POST /events`** mapeia o `hook_event_name` pro estado:

| Evento                  | Vira                                                |
|-------------------------|-----------------------------------------------------|
| `UserPromptSubmit`      | 🟡 working                                          |
| `Notification`          | 🔴 waiting (pergunta genérica)                      |
| `Stop`                  | 🟢 ready, lendo a última mensagem do transcript     |
| `SessionEnd`            | remove a sessão                                     |

A pasta vem do basename do `cwd`. O `container` é inferido pelo header
`X-Semaforo-Container: 1` que o hook manda, ou, como fallback, por o IP de origem
não ser loopback; uma vez detectado como container, fica travado, pra um evento
do host depois não fazer o badge piscar. A última mensagem no `Stop` é lida do
`transcript_path` quando ele é acessível do host (melhor esforço; em container
cai num texto genérico), confinada a `~/.claude/projects/` e lendo só o fim do
arquivo.

**`POST /permission`** é o pulo do gato. Recebe o payload de um hook `PreToolUse`,
extrai o comando/ferramenta, e:

1. Se o comando casar com uma regra "Sempre" (`allow_rules`), responde `allow` na
   hora, sem te incomodar.
2. Se "responder pela pílula" estiver desligado, responde `ask` (cai no prompt
   nativo do Claude Code).
3. Senão, marca a sessão como 🔴 com o comando pendente, cria um canal, guarda o
   `Sender` em `pending[session_id]`, emite o snapshot, e **bloqueia** num
   `recv_timeout(600s)` esperando a decisão.

Quando você clica na pílula, o comando `respond` manda a `Decision` pelo canal, e
a thread do HTTP acorda e devolve:

```json
{ "hookSpecificOutput": {
    "hookEventName": "PreToolUse",
    "permissionDecision": "allow" | "deny" | "ask",
    "permissionDecisionReason": "respondido pelo Claude Semáforo" } }
```

`allow` libera a tool de verdade, `deny` bloqueia, `ask` cai no prompt nativo. Se
estourar o timeout de 600s sem decisão, responde `ask` **e** reseta a sessão pra
🟡 working ("Tempo esgotado — responda no terminal."), pra pílula não ficar com um
🔴 fantasma. O timeout padrão do `PreToolUse` no Claude Code é 600s, então tem
folga pra decisão humana.

### Comandos (`commands.rs`)

- `get_state` → devolve o `Snapshot` atual.
- `respond(session_id, decision)` → `"allow" | "deny" | "always"`. Atualiza a
  sessão pra working, manda a decisão pela request segurada, e no caso de
  `always` grava (e persiste) a regra de auto-permitir.
- `get_config` / `set_config(patch)` → lê/grava a config. `set_config` trata os
  efeitos colaterais: trocar o bind reinicia o servidor, mexer no "sempre no
  topo" chama a janela, ligar/desligar o autostart chama o plugin.
- `regenerate_token` / `reveal_token` → gera um token novo / devolve o atual (pro
  botão de copiar). Ao regenerar, sincroniza o `~/.claude/semaforo.token` se os
  hooks já estiverem instalados.
- `save_window(x, y)` → persiste o canto da janela.
- `install_hooks` / `hooks_installed` → escreve `notify.sh`/`notify.ps1` + token em
  `~/.claude` e mescla os cinco hooks no `~/.claude/settings.json`, preservando os
  seus hooks e sendo idempotente (a lógica de merge é pura e testada). O comando
  monta o comando certo por SO: `bash notify.sh` no Linux, `powershell ... notify.ps1`
  no Windows.
- `quit_app` → encerra o app.

## Setup automático e como fechar (setup.rs + tray)

Como a janela não tem entrada na barra de tarefas nem botão de fechar, tem uma
**system tray**: clique esquerdo abre/fecha o painel, clique direito tem **Sair**.
A config também tem um botão **Instalar** (liga os hooks do Claude Code de uma vez)
e um **Sair do Claude Semáforo**. O token vai pra um arquivo `~/.claude/semaforo.token`
que os scripts leem, então regenerar o token não exige reinstalar. Containers e
workspaces remotos continuam no passo manual (o app escreve no `~/.claude` do host).

### Persistência e config (`config.rs`)

A config é um JSON em `app_config_dir()/config.json`. Na primeira execução, gera
um token aleatório (`csf_` + 16 bytes do RNG do sistema, em hex). As variáveis de
ambiente `SEMAFORO_TOKEN` e `SEMAFORO_BIND` sobrescrevem em runtime. As regras de
"Sempre permitir" vivem num sidecar próprio (`allow_rules.json`), separado da
config pra que rotação de token não cause churn no arquivo de regras, e são
carregadas no boot — então um "Sempre" sobrevive a reinício. No Unix, o
`config.json` (que embute o token) e o `~/.claude/semaforo.token` são gravados com
permissão `0o600`.

### Limpeza (sweeper)

Uma thread roda a cada 30s e remove sessões sem update há mais de 15 minutos, pra
lista não acumular sessões zumbis. Emite um snapshot novo quando algo muda.

### Janela e plugins (`lib.rs`)

A `tauri.conf.json` cria a janela `decorations:false`, `transparent:true`,
`alwaysOnTop:true`, `skipTaskbar:true`, começando invisível e no tamanho
encolhido. No `setup`, o backend carrega a config, sobe o servidor, posiciona a
janela (canto salvo ou inferior direito do monitor) e a torna visível. Plugins:
`opener` (abrir links), `notification` (aviso do SO quando vira 🔴) e `autostart`
(iniciar com o sistema). As permissões da janela usadas pelo frontend
(`set-size`, `set-position`, `current-monitor`, etc.) estão em
`capabilities/default.json`.

## Hooks do Claude Code (`hooks/`)

- **`notify.sh`** (Linux / dentro de container) e **`notify.ps1`** (Windows
  nativo) leem o JSON do hook no stdin. Se for `PreToolUse`, mandam pra
  `/permission` e imprimem a resposta no stdout (é assim que o Claude Code recebe
  a decisão de permissão). Qualquer outro evento vai pra `/events`. Tentam
  `host.docker.internal:7337` primeiro, depois `127.0.0.1:7337`.
- **`.claude/settings.local.example.json`** registra os quatro hooks de estado
  mais o `PreToolUse` de permissão. É `.example` de propósito, pra você copiar pra
  `settings.local.json` e não sujar o versionado nem mexer nesta própria sessão.

### Travessia container → host

- Docker Desktop: `host.docker.internal` já resolve.
- Docker no Linux: precisa de `"runArgs": ["--add-host=host.docker.internal:host-gateway"]`
  no `devcontainer.json` (por isso o app escuta em `0.0.0.0`).
- Workspace remoto: aponte `SEMAFORO_URL` pra um túnel de volta pro widget.

## Segurança

- Token Bearer obrigatório em todo request, comparado em tempo constante. Nunca
  vai no snapshot emitido à webview; a UI o busca sob demanda via `reveal_token`.
- Bind configurável: `0.0.0.0:7337` alcança containers; `127.0.0.1:7337` tranca
  tudo no host.
- Body dos POSTs com teto (256 KB no `/permission`, 64 KB no `/events`) pra um
  container autenticado-mas-hostil não derrubar o widget por OOM.
- A regra de "Sempre permitir" usa a identidade precisa da chamada (caminho
  completo, args canônicos), não um rótulo truncado — então aprovar uma chamada
  não libera outra parecida.
- Leitura de `transcript_path` confinada a `~/.claude/projects/` (sem `..`).
- No Unix, token e config ficam `0o600`.
- A primeira execução no Windows pode pedir liberação da porta no firewall.

## Build e release

- `npm run tauri dev` / `npm run tauri build` localmente.
- `.github/workflows/release.yml` builda por tag `vX.Y.Z`: no Ubuntu sai `.deb` e
  `.AppImage`, no Windows sai `.exe` (NSIS) e `.msi`, via `tauri-action`, num
  release em rascunho.

## Testes

- Backend: `cargo test` cobre as funções puras (basename, rótulo vs. chave da
  tool, comparação em tempo constante, parsing/confinamento do transcript, teto de
  body, serialização camelCase da config, snapshot sem token, mapeamento de
  evento → estado, latch do container, não-flash do `has_pending`, reset no
  timeout, persistência das regras).
- Frontend: `npm test` (vitest) cobre o `derive()` (pior estado, contadores,
  subtítulo), o `nextCue()` (cue sonoro, waiting na primeira aparição) e o
  `relTime()`.

## Decisões e pontos de atenção

- **`tiny_http` em vez de axum**: menos dependências, e o modelo de uma thread por
  request casa bem com o long-poll do `/permission`.
- **Sessões em `Ask`** (vindas de `Notification`) não têm canal de volta com o
  protocolo de hooks atual — o `/events` é fire-and-forget. A pílula só aponta
  "responda no seu terminal". Responder em texto exigiria um hook que segura a
  resposta, como o `/permission` faz.
- **Fontes via Google Fonts** (`@import` no CSS) com fallback pro sistema. Pra uso
  100% offline, o próximo passo é empacotar as fontes localmente.
- **Detecção de container** é heurística (header do hook + IP de origem). O header
  é o sinal confiável; o IP é o fallback.
- O protótipo de design mostrava também um "desktop" de fundo (wallpaper, janela
  do editor, taskbar) só pra encenação. O app real é só a pílula + painel numa
  janela transparente.
