# Ecossistema Lucerna

Este documento descreve como os repositórios em Rust se encaixam no [Lucerna](https://github.com/EduardoSA8006/lucerna). Ele só registra o que já foi decidido; o que falta definir está em "Em aberto".

## Repositórios

Um repositório por produto, mais o `lucerna-sdk` como base comum. Todos têm as mesmas configurações do `lucerna`: licença GPL-3.0, branch `main` protegida por ruleset, merge só por merge commit, PR obrigatório e padrão de commits em português.

| Repositório | Papel |
|---|---|
| [`lucerna`](https://github.com/EduardoSA8006/lucerna) | O shell, em Quickshell (QML). Só a interface. |
| [`lucerna-sdk`](https://github.com/EduardoSA8006/lucerna-sdk) | Base comum: o protocolo de conversa com o shell e o manifesto dos apps. |
| [`lucernad`](https://github.com/EduardoSA8006/lucernad) | Daemon com a lógica do shell (espectro, sistema, área de transferência e mais). |
| [`lucerna-phone`](https://github.com/EduardoSA8006/lucerna-phone) | Gerenciador de celulares. |
| [`lucerna-phone-android`](https://github.com/EduardoSA8006/lucerna-phone-android) | App Android do `lucerna-phone`. |
| [`lucerna-routine`](https://github.com/EduardoSA8006/lucerna-routine) | Gerenciador de rotina. |
| [`lucerna-notes`](https://github.com/EduardoSA8006/lucerna-notes) | Bloco de notas. |

## O shell é só a interface

O `lucerna` (Quickshell/QML) cuida da interface: painéis, temas, animações e as APIs nativas do Quickshell. A lógica fica em Rust, nos outros repositórios.

## Estrutura de cada app

Cada app tem quatro partes, todas no repositório dele:

1. **Núcleo:** uma biblioteca com a lógica.
2. **Serviço:** um processo sem tela que guarda o estado, faz o trabalho e atende o socket. É ele que conversa com o shell.
3. **TUI:** uma interface de terminal, feita com ratatui.
4. **Painel:** o próprio painel, em QML.

A TUI e o painel são clientes do serviço. Nenhum dos dois mexe nos dados direto.

O pacote do app instala o painel e um manifesto numa pasta que o shell conhece, por exemplo `/usr/share/lucerna/apps/*.json`.

## Host de painéis no shell

O shell terá um "host de painéis":

- lê os manifestos e carrega os painéis;
- oferece uma API de componentes QML estável e versionada, com os widgets e o tema do Lucerna;
- confere no manifesto para qual versão da API o painel foi feito e recusa, com um aviso, o que não consegue carregar.

Sem o app instalado, o painel não existe.

Os painéis dos apps abrem como os de hoje, por atalho, IPC ou ícone na barra, e seguem a regra de fechar os outros painéis. Isto é uma suposição aceita, ainda sem implementação.

## Protocolo (proposta)

Proposta, ainda a definir:

- um socket Unix por app, em `$XDG_RUNTIME_DIR/lucerna/<app>.sock`;
- mensagens JSON versionadas;
- atualizações enviadas pelo app.

O `lucerna-sdk` traz os crates do protocolo e do manifesto, publicados no crates.io.

## Serviço sob demanda

O serviço é a única parte que mexe nos dados do app. Isso vale para o painel e para a TUI:

- **Os dois são clientes do serviço.** Como ninguém mexe nos dados por fora, os dois nunca brigam pelo mesmo dado, e o que muda num aparece na hora no outro.
- **A TUI é separada do painel e é uma alternativa a ele.** O shell nunca abre a TUI para conversar com o app.
- **O painel abre o app sozinho, sem TUI.** Ele conecta ao socket, e o serviço sobe por ativação de socket do systemd do usuário (`<app>.socket` + `<app>.service`). Se o serviço não estiver rodando, a TUI o sobe pelo mesmo caminho.
- **O serviço não fica rodando o tempo todo.** Ele sobe quando um cliente conecta, sai sozinho depois de um tempo sem clientes e sem trabalho pendente e, se cair, o systemd reinicia.

**Trabalho agendado sem o serviço rodando (proposta):** um lembrete da rotina, por exemplo, precisa acontecer com o serviço parado. Então o serviço registra um timer do systemd do usuário para o próximo horário. O timer sobe o serviço, que avisa pelas notificações do shell e volta a sair. O tempo de ociosidade e o mecanismo exato do timer estão em aberto.

## Painel e TUI

O painel no shell serve para **olhar rápido e agir rápido**, sem tirar o usuário do que está fazendo. A TUI é o lugar do **trabalho completo**: editar, organizar e configurar.

## Pastas e dados

Por enquanto, os dados ficam só neste computador. As pastas seguem o padrão XDG:

| Tipo | Onde | Exemplo |
|---|---|---|
| Documentos do usuário (o que ele abre e edita) | pasta visível e configurável | `~/Documentos/Notas` |
| Dados internos | `~/.local/share/lucerna/<app>/` (`$XDG_DATA_HOME`) | banco da rotina, chaves de pareamento do celular |
| Configuração | `~/.config/lucerna/<app>.toml` (`$XDG_CONFIG_HOME`) | preferências |
| Cache (pode ser apagado e se refaz) | `~/.cache/lucerna/<app>/` (`$XDG_CACHE_HOME`) | índice de busca das notas, miniaturas |
| Sockets | `$XDG_RUNTIME_DIR/lucerna/<app>.sock` | painel e TUI |

- Cada app só mexe na própria subpasta.
- O nome `lucerna` vale mesmo quando o app roda só na TUI.
- O `lucerna-sdk` traz essas regras de caminho num crate pequeno, para os apps não divergirem.

## O que o SDK oferece

Estas capacidades vêm dos casos dos apps:

1. **Estado enviado pelo app:** a bateria do celular, a próxima tarefa, as notas recentes. O painel só mostra.
2. **Ações chamadas pelo painel:** "marcar tarefa", "salvar nota", "espelhar", com resposta de sucesso ou erro.
3. **Itens na barra:** um ícone com um número ou um timer que o app põe na barra do shell (o pomodoro, o celular conectado).
4. **Notificações:** pelo servidor de notificações que o shell já tem, sem nada novo.
5. **Abrir na TUI:** o painel pode pedir ao shell para abrir o terminal já no app, na nota ou tarefa certa, **para o usuário**. É um atalho para o trabalho completo. A comunicação com o app nunca depende da TUI.
6. **Dois tamanhos:** o painel do app, que abre como os outros painéis, e, opcional, um card pequeno na aba Painel do dashboard do shell.
7. **Caminhos:** um crate pequeno com as regras de pastas de "Pastas e dados" (dados, configuração, cache e sockets), para os apps não divergirem.

## Em aberto

- O formato exato do protocolo e do manifesto, incluindo as mensagens de estado, de ação, dos itens na barra, das notificações e do pedido para abrir na TUI.
- O versionamento: do protocolo, do manifesto e da API de componentes QML.
- O empacotamento, incluindo o AUR.
- A forma do `lucernad`: um daemon com módulos ou vários binários.
- O tempo de ociosidade do serviço antes de sair.
- O mecanismo dos timers do systemd para o trabalho agendado.
- O que o SDK oferece para o serviço sob demanda: um crate com a ativação por socket e a saída por ociosidade?

## Futuro

- A sincronização dos dados entre aparelhos. Por enquanto, os dados ficam só neste computador.
