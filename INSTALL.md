# Guia de instalação

Passo a passo para colocar o **Claude Semáforo** pra rodar e ligar nos seus
agentes do Claude Code. Se você só quer a versão curta, ela está no
[README](./README.md). Aqui vai com mais detalhe e resolução de problemas.

## 1. Baixar o instalador

Pegue o arquivo do seu sistema na página de
[Releases](https://github.com/julianosirtori/claude-semaforo/releases):

| Sistema           | Arquivo                              |
| ----------------- | ------------------------------------ |
| Windows           | `.exe` (instalador NSIS) ou `.msi`   |
| Linux (Debian)    | `.deb`                               |
| Linux (qualquer)  | `.AppImage`                          |

No Windows, abra o `.exe` e siga o instalador. No Linux, instale o `.deb`
(`sudo dpkg -i claude-semaforo_*.deb`) ou marque o `.AppImage` como executável
e rode (`chmod +x *.AppImage && ./Claude\ Semáforo*.AppImage`).

> Na primeira execução o Windows pode pedir para liberar a porta no firewall.
> Aceite: o app escuta em `7337` e é isso que permite que as sessões (inclusive
> em containers) reportem o estado.

## 2. Abrir o widget

O Claude Semáforo é uma pílula sempre no topo, sem entrada na barra de tarefas.
Ao abrir, ela aparece num canto da tela. Clique na pílula para abrir o painel
com a lista de sessões por projeto.

Para **fechar o app**: clique direito no ícone da bandeja do sistema (system
tray) → **Sair**, ou pela pílula → engrenagem → **Sair do Claude Semáforo**.
Clique esquerdo no ícone da bandeja abre e fecha o painel.

## 3. Ligar os hooks do Claude Code

O widget só mostra alguma coisa depois que o Claude Code começa a reportar os
eventos. Isso é feito por hooks.

### Jeito fácil (host, máquina nativa)

1. Abra a pílula → **engrenagem** (Configuração).
2. Vá em **Claude Code** e clique em **Instalar**.

Isso escreve `notify.sh` e `notify.ps1` em `~/.claude/`, grava o token em
`~/.claude/semaforo.token` e mescla os cinco hooks no `~/.claude/settings.json`,
preservando qualquer hook que você já tenha. Pronto: as próximas sessões do
Claude Code já reportam aqui.

> Regenerar o token (Configuração → Conexão → regenerar) **não** exige
> reinstalar. O token novo é sincronizado no arquivo automaticamente.

### Jeito manual (containers e workspaces remotos)

O botão acima escreve no `~/.claude` do **host**. Dentro de um container você
faz na mão:

1. Copie o script de hook para junto da sua config do Claude:

   ```bash
   cp hooks/notify.sh ~/.claude/           # Linux / macOS / containers
   # Windows nativo:  copy hooks\notify.ps1 %USERPROFILE%\.claude\
   ```

2. Exporte o token que aparece em **Configuração → Token** (botão de copiar):

   ```bash
   export SEMAFORO_TOKEN="csf_...."
   ```

3. Registre os hooks. Copie `.claude/settings.local.example.json` para
   `.claude/settings.local.json` (por projeto) ou para `~/.claude/settings.json`
   (global). Isso liga os quatro eventos de estado mais o hook de permissão
   `PreToolUse`. No Windows nativo, troque o comando por
   `powershell -NoProfile -File "%USERPROFILE%\.claude\notify.ps1"`.

#### Alcançar o host a partir de um container

- **Docker Desktop** — `host.docker.internal` já resolve, nada a fazer.
- **Docker no Linux** — adicione ao seu `devcontainer.json`:

  ```json
  "runArgs": ["--add-host=host.docker.internal:host-gateway"]
  ```

- **Workspace remoto (Codespaces / SSH)** — aponte `SEMAFORO_URL` para um túnel
  de volta ao widget, por exemplo `export SEMAFORO_URL="http://127.0.0.1:7337"`.

O hook tenta `host.docker.internal` primeiro e cai para `127.0.0.1`. Sessões em
container ganham o selo `container` no widget.

## 4. Conferir que funcionou

1. Com o widget aberto, rode qualquer comando numa sessão do Claude Code.
2. A pílula deve mudar de cor:
   - 🟡 **Trabalhando** — Claude está pensando.
   - 🔴 **Te esperando** — parou para pedir permissão ou fazer uma pergunta.
   - 🟢 **Pronto** — terminou, tem saída para revisar.
3. Numa sessão 🔴 você responde direto na pílula: **Permitir** / **Sempre** /
   **Negar**. A decisão volta para o Claude Code e o comando é de fato liberado
   ou bloqueado.

Se nada acontecer, confira a seção de problemas abaixo.

## 5. Build a partir do código (opcional)

Precisa de Node 20+ e da toolchain do Rust (mais as dependências de sistema do
Tauri).

```bash
npm install
npm run tauri dev      # roda o widget em desenvolvimento
npm run tauri build    # gera os instaladores em src-tauri/target/release/bundle
```

## Problemas comuns

- **A pílula não muda de estado.** O Claude Code não está alcançando a porta.
  Confira se os hooks foram instalados (Configuração → Claude Code mostra
  "instalados") e se o token na sessão bate com o do widget.
- **Em container, nada chega.** Verifique se `host.docker.internal` resolve de
  dentro do container e se a porta `7337` está acessível a partir dele.
- **`401`/sem resposta nos hooks.** O `SEMAFORO_TOKEN` exportado na sessão não
  confere com o token atual do widget. Copie o token de novo em Configuração →
  Token, ou reinstale os hooks pelo botão (que regrava o `semaforo.token`).
- **Não acho como fechar.** Use a bandeja do sistema (clique direito → Sair) ou
  a engrenagem → Sair do Claude Semáforo. Não há botão na barra de tarefas.

Para o desenho completo do sistema, veja [ARCHITECTURE.md](./ARCHITECTURE.md).
