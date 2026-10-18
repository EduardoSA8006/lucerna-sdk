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
2. **TUI:** uma interface de terminal que funciona sozinha, feita com ratatui.
3. **Provedor:** o processo que conversa com o shell.
4. **Painel:** o próprio painel, em QML.

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

## Em aberto

- O formato exato do protocolo e do manifesto.
- O versionamento: do protocolo, do manifesto e da API de componentes QML.
- O empacotamento, incluindo o AUR.
- A forma do `lucernad`: um daemon com módulos ou vários binários.
