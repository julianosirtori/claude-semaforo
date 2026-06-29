# Review: sincronização app ↔ Claude Code

**Data:** 2026-06-29
**Escopo:** app inteiro, com foco na sincronização entre o widget e o Claude Code. Inclui as mudanças não commitadas da feature de som (`src/sound.ts` + integração).
**Método:** review autônomo — revisores especializados (segurança + qualidade/performance) em paralelo, seguidos de auditoria red-team que verificou cada achado contra o código real, descartou falsos positivos e recalibrou severidades.

---

## Sumário executivo

O estado que o pill mostra é uma **réplica em memória** alimentada por eventos *fire-and-forget* vindos dos hooks do Claude Code. Há quatro pontos onde essa réplica diverge do estado real. Dois são bugs; dois são limitações de design:

1. **Timeout de permissão deixa a sessão presa em 🔴** (o mais visível — bug).
2. **Eventos de estado podem se perder** (`/events` com `curl -m 3`, sem retry nem reconciliação) → sessão fica 🟡/🟢 errada por até 15 min (design).
3. **Reinício do app zera o estado** — sessões só reaparecem no próximo evento; `allow_rules` somem (design/bug).
4. **`reply_text` é um toggle que não faz nada** (bug).

O núcleo está bem arquitetado: o long-poll de permissão, o gating por `permission_mode` e o merge idempotente de hooks são sólidos. Mas a sincronização tem dois furos reais e o "Sempre permitir" concede largo demais e não persiste.

**Veredito:** precisa de ajustes (2 achados Alto, 3 Médio, 6 Baixo). Nenhum Crítico.

---

## Achados priorizados

| # | Sev | Tag | Achado | Local |
|---|-----|-----|--------|-------|
| 1 | Alto | quality | Timeout do `/permission` deixa a sessão presa em `waiting` | `server.rs:376-383` |
| 2 | Alto | security | `describe_tool` descarta contexto → allow-rules concedem demais | `server.rs:421-439` |
| 3 | Médio | quality | `allow_rules` não é persistido | `lib.rs:49`, `config.rs` |
| 4 | Médio | quality | `reply_text` morto e inerte | `commands.rs:62-66`, `SessionRow.tsx` |
| 5 | Médio | security | Body dos POSTs sem limite de tamanho | `server.rs:147` |
| 6 | Baixo | quality | Som não toca na primeira aparição em waiting/ready | `App.tsx:50-54` |
| 7 | Baixo | security | Token vai em todo `emit("snapshot")` | `state.rs:59,138` |
| 8 | Baixo | security | `transcript_path` lê arquivo arbitrário | `server.rs:442-444` |
| 9 | Baixo | security | Token em arquivo world-readable no Unix | `setup.rs:115`, `config.rs:51` |
| 10 | Baixo | quality | `SubagentStop` é código morto | `server.rs:269` vs `setup.rs:21-27` |
| 11 | Baixo | cosmético | Flash sem mudança quando `has_pending` bloqueia; flag `container` oscila | `server.rs:248-251` |

---

## Alto

### 1. Timeout do `/permission` deixa a sessão travada em `waiting`

**`src-tauri/src/server.rs:376-383`** · `[quality]`

```rust
let decision = rx.recv_timeout(PERMISSION_TIMEOUT).unwrap_or(Decision::Ask);
if let Decision::Ask = decision {
    // Timed out: drop our responder slot so it doesn't linger.
    if let Ok(mut g) = inner.lock() {
        g.pending.remove(&session_id);
    }
}
let _ = req.respond(permission_response(decision));
```

Quando os 600s expiram, o código remove o `pending` e responde `Ask` ao Claude Code, mas **não** atualiza o estado da sessão nem chama `emit`. A sessão fica em `Waiting`/`ReqKind::Perm` com o `cmd` antigo, `updated_at` não é bumpado (sem flash), e a UI nunca é notificada. O pill mostra um 🔴 fantasma até:
- chegar um `PostToolUse` (só se o usuário respondeu no terminal), ou
- o sweeper limpar a sessão em ~15 min.

É a causa direta da sensação de "sync não está 100%".

**Teste (RED primeiro):** segura uma permissão, força o timeout (reduzir `PERMISSION_TIMEOUT` para 1s num teste, ou fatorar a lógica de reset para uma função pura testável), e afirma que o estado virou `Working`, `req_kind` é `None`, `updated_at` foi bumpado e um `emit` foi disparado.

**Fix:** no ramo de timeout, além de remover o `pending`, resetar a sessão para `Working` com mensagem "Tempo esgotado — responda no terminal.", bumpar `updated_at` e chamar `emit(app, inner)`. `app` já está no escopo da função.

---

### 2. `describe_tool` descarta o contexto, então "Sempre permitir" concede demais

**`src-tauri/src/server.rs:421-439`** · `[security]`

A chave da regra de auto-allow (`allow_rules`) é gerada por `describe_tool` e perde informação crítica:

- **Tools MCP** (casados por `mcp__.*` em `PERM_MATCHER`): não têm `file_path`/`path`, caem no fallback `raw = tool_name.to_string()`. Clicar "Sempre" em `mcp__github__create_pull_request` uma vez auto-aprova **toda** chamada futura desse tool, com qualquer payload, para sempre.
- **Write/Edit/Read/MultiEdit**: a chave é `format!("{tool_name} {}", basename(p))` — sem diretório. Aprovar `Write .env` num projeto **auto-permite sobrescrever `.env` em todos os outros projetos**.
- **Bash**: comandos com mais de 120 chars são truncados a 117 + "…" (`server.rs:433-438`). Dois comandos com o mesmo prefixo de 117 chars colidem na mesma regra.

A chave armazenada (`commands.rs:26-31`, via `session.cmd` setado em `server.rs:353`) e a comparada (`server.rs:320`) usam a mesma função, então o match é consistente — mas semanticamente largo demais. O red-team rebaixou a variante Bash (exige payload coludido com o LLM; o residual é colisão acidental) e **elevou** MCP/arquivo: é escalada de privilégio silenciosa e realista, exatamente o tipo de coisa que morde um usuário casual antes de qualquer ataque.

**Teste (RED primeiro):** allow `Write` em `/a/.env`; depois uma nova `/permission` de `Write` em `/b/.env` **não** pode bater na regra (`g.allow_rules.contains(&cmd)` deve ser `false`).

**Fix:** separar duas responsabilidades hoje fundidas em `describe_tool`:
- `describe_key` — sem truncar; inclui o diretório para file-tools e os args canônicos (ou um hash estável deles) para MCP/Bash. Usado para gravar e comparar `allow_rules`.
- `describe_label` — a versão truncada/amigável atual. Usado só para exibição (`s.cmd`, `s.last_msg`).

---

## Médio

### 3. `allow_rules` não é persistido

**`src-tauri/src/lib.rs:49`, `src-tauri/src/config.rs`** · `[quality]`

`allow_rules: HashSet::new()` é recriado a cada boot. `allow_rules` vive em `Inner`, não em `Config`, e `config.rs` só faz round-trip de `Config`. Nada salva nem carrega as regras. Todo restart do widget apaga todos os "Sempre permitir" — quem clicou "Sempre" em `npm run migrate:prod` é perguntado de novo.

**Fix:** persistir as regras. Preferir um sidecar (`allow_rules.json`) ou um struct próprio, **separado de `Config`**, para que rotação de token não cause churn no arquivo de regras. Teste: inserir uma regra, simular reload, esperar a regra presente.

---

### 4. `reply_text` está morto e inerte

**`src-tauri/src/commands.rs:62-66` + `src/components/SessionRow.tsx`** · `[quality]`

```rust
s.last_msg = if text.trim().is_empty() {
    "Voltando ao trabalho…".into()
} else {
    "Voltando ao trabalho…".into()   // ramo idêntico — texto descartado
};
```

Dois problemas:
1. Os dois ramos do `if/else` produzem a mesma string; o texto do usuário é descartado silenciosamente.
2. Mais grave: **nenhum** evento cria canal `pending` para sessões em `Ask` — só `/permission` insere em `pending` (`server.rs:362`). `Notification` (que gera `ReqKind::Ask`) não. Então `g.pending.remove(&session_id)` em `commands.rs:69` sempre retorna `None`, e o texto **nunca** chega ao Claude Code.

A UI (campo, Enter, botão Enviar) existe e é gated por `cfg.replyText`, mas é estruturalmente inativa com o protocolo de hooks atual (o `/events` é fire-and-forget, não segura resposta).

**Fix:** decidir entre os dois caminhos, sem enviar o meio-termo:
- **Remover** o toggle, o branch da UI e o comando; ou
- **Construir de verdade** — exigiria um evento de hook que *segura* a resposta (como o `/permission` faz), não o `/events` atual.

---

### 5. Body dos POSTs sem limite de tamanho

**`src-tauri/src/server.rs:147`** · `[security]`

```rust
let mut body = String::new();
let _ = std::io::Read::read_to_string(req.as_reader(), &mut body);
```

`read_to_string` lê até EOF sem cap, e o servidor é um thread por request (`server.rs:59`). Um container comprometido — que legitimamente tem o token — pode mandar vários POSTs concorrentes de centenas de MB e derrubar o widget por OOM. A leitura sem cap também afeta `last_assistant_message` (`fs::read_to_string` em `server.rs:444`). Token-gated, por isso Médio.

**Fix:** `req.as_reader().take(MAX_BODY).read_to_string(...)` com, p.ex., 256 KB para `/permission` e 64 KB para `/events`.

---

## Baixo

### 6. Som não toca na primeira aparição em waiting/ready

**`src/App.tsx:50-54`** · `[quality]` · (feature nova)

A guarda `wasState !== undefined` impede o cue sonoro quando o primeiro evento de uma sessão já é `PreToolUse` (sessão resumida com `--continue`, ou que existia antes do app abrir, ou que reapareceu após o sweeper). No fluxo normal, `UserPromptSubmit` (working) precede o `PreToolUse`, então a sessão já existe em `prevState` quando vira waiting — por isso o caso é incomum.

**Fix:** tratar `wasState === undefined && s.state === "waiting"` como cue (waiting na primeira aparição já merece aviso). Opcional para `ready`.

### 7. Token vai em todo `emit("snapshot")`

**`src-tauri/src/state.rs:59,138`** · `[security]`

`Config` serializa `token`, e o `Snapshot` carrega o `Config` inteiro, emitido à webview a cada mudança de estado (`server.rs:120-124`, `commands.rs:10-13`). O consumidor é a própria webview (mesmo processo, sem IPC remoto), então não é vazamento de rede — mas o token aparece em payloads de evento, visíveis em DevTools ou captura de log. `reveal_token` já existe para busca sob demanda.

**Fix:** `#[serde(skip_serializing)]` no campo `token` (ou um `PublicConfig` sem token para o snapshot). A UI continua buscando via `reveal_token` no copiar.

### 8. `transcript_path` lê arquivo arbitrário

**`src-tauri/src/server.rs:442-444`** · `[security]`

```rust
let path = str_field(payload, "transcript_path")?;
let content = fs::read_to_string(path).ok()?;
```

O caminho vem do body autenticado; lê qualquer arquivo acessível ao processo. Mitigado: token-gated, a saída é filtrada a linhas JSONL com `type: "assistant"` e cortada a 140 chars, então não serve para exfiltrar segredos arbitrários. Sem cap de tamanho na leitura (mesmo ponto do #5).

**Fix:** restringir a um prefixo conhecido (ex.: `~/.claude/projects/`), ou remover — `Stop` já pode fornecer `message` direto.

### 9. Token em arquivo world-readable no Unix

**`src-tauri/src/setup.rs:115`, `src-tauri/src/config.rs:51-53`** · `[security]`

`semaforo.token` e `config.json` (que também serializa o token) são escritos sem permissões explícitas, herdando o umask (tipicamente 644 = world-readable). Em máquina Linux compartilhada ou container multi-usuário, outro usuário lê o token e, com o bind `0.0.0.0`, alcança o servidor. `notify.sh` já seta 0o755 no script, mas deixa o token intocado. (No host Windows nativo não morde, mas o app é cross-platform.)

**Fix:** `set_permissions(..., 0o600)` após escrever o token e o config (sob `#[cfg(unix)]`).

### 10. `SubagentStop` é código morto

**`src-tauri/src/server.rs:269` vs `src-tauri/src/setup.rs:21-27`** · `[quality]`

`apply_event` trata `"Stop" | "SubagentStop"` identicamente, mas `STATE_EVENTS` não inclui `SubagentStop`, então o hook nunca é registrado e o evento nunca chega. Se fosse registrado sem pensar, marcaria a sessão pai como 🟢 *ready* enquanto ela ainda roda subagentes (Task). `ARCHITECTURE.md:162` documenta ambos — a omissão é drift não documentado.

**Fix:** decidir — remover o braço do match e a menção no doc, ou registrar `SubagentStop` conscientemente (provavelmente mapeando para algo que **não** seja `Ready`).

### 11. Flash sem mudança / flag `container` oscila

**`src-tauri/src/server.rs:248-251`** · `[cosmético]`

- Um `Notification` numa sessão com permissão segurada pula a troca de estado (correto, via `has_pending`), mas ainda faz `entry.updated_at = now_ms()` e emite → a linha pisca sem mudança semântica.
- `entry.container` é sobrescrito a cada evento; se eventos vierem de origens diferentes (ex.: `/permission` de container e depois um `Notification` do host), o badge oscila e mente sobre o que a sessão é.

**Fix:** só bumpar `updated_at` quando algo de fato mudou; congelar `container` após a primeira definição (ou usar OR lógico).

---

## Bons padrões (confirmados pela auditoria)

- **`PostToolUse` destrava só sessões presas em `waiting`** sem bumpar `updated_at` no caso normal — preserva a detecção de flash. Bem coberto por testes (`server.rs:588-613`).
- **Guarda `has_pending`** impede um `Notification` de rebaixar uma permissão segurada (`Perm` → `Ask`). Fixado pelo teste `notification_keeps_held_permission`.
- **`constant_time_eq`** na comparação do token; `!token.is_empty()` evita que token vazio case com qualquer coisa.
- **`build_settings` puro e idempotente** — merge por filter+push, com três testes bem escolhidos (from-empty, preserva hooks do usuário, idempotência).
- **Gating de `permission_mode`** — modos `auto`/`bypass`/`plan` são deferidos para `Ask` antes de mexer no estado, por isso o modo auto nunca mostra 🔴 falso.
- **CSRF/drive-by de browser barrado** — o header `Authorization` força preflight CORS, que cai em 404 no tiny_http; form submissions não carregam o header.
- **Interpolação do token no bootstrap do devcontainer é segura** — alfabeto hex (`csf_` + hex), aspas simples no shell não podem ser fechadas pelo conteúdo.
- **Substituição de canal pending é segura** — uma segunda `/permission` para a mesma sessão substitui e dropa o `Sender` antigo; o `recv_timeout` original recebe `Disconnected` e resolve para `Ask` (fallback seguro).

---

## Notas de auditoria (recalibrações e falsos positivos)

- **Colisão de truncamento Bash:** Crítico → **Médio**. Exige payload coludido com o LLM (que autora o comando); o residual é colisão acidental improvável. O problema real de allow-rules é o **MCP/arquivo** (achado #2).
- **Token no snapshot:** Alto → **Baixo**. Consumidor é a própria webview; `reveal_token` já existe. Vira problema só se houver XSS na webview.
- **Leitura de `transcript_path`:** Alto → **Baixo**. Token-gated, saída filtrada e cortada.
- **Bind `0.0.0.0`:** **não** é vulnerabilidade — é trade-off documentado para alcançar containers (`host.docker.internal`). Protegido por token.

---

## Recomendação de ordem de correção

Atacar nesta ordem resolve o grosso da queixa de sincronização:

1. **#1** — timeout preso em waiting (resolve o sintoma mais visível).
2. **#2** — chave de allow-rules (segurança real).
3. **#3** — persistir allow_rules.
4. Depois: **#4** (decidir o destino do `reply_text`), **#5**, e os Baixos conforme houver fôlego.

Todos os fixes de backend devem seguir o ciclo TDD do projeto (RED primeiro), aproveitando que `apply_event` e a lógica de config já são testáveis de forma pura.

---

## Apêndice: arquivos revisados

**Backend:** `server.rs`, `commands.rs`, `state.rs`, `lib.rs`, `setup.rs`, `config.rs`
**Hooks:** `hooks/notify.sh`, `hooks/notify.ps1`
**Frontend:** `src/App.tsx`, `src/api.ts`, `src/types.ts`, `src/sound.ts`, `src/components/ConfigView.tsx`, `src/components/Panel.tsx`, `src/components/SessionRow.tsx`
**Docs:** `ARCHITECTURE.md`, `CLAUDE.md`
