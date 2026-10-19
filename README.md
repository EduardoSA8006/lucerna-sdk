# lucerna-sdk

Base comum do ecossistema Lucerna em Rust: o protocolo de conversa com o shell e o manifesto dos apps.

**Situação:** em documentação; ainda sem código.

## Ligação com o Lucerna

Faz parte do ecossistema do [Lucerna](https://github.com/lucerna-desktop/lucerna-shell), o shell de desktop para o Hyprland. O shell cuida só da interface; a lógica fica em Rust, neste e nos outros repositórios do ecossistema (veja [`docs/ecossistema.md`](https://github.com/lucerna-desktop/lucerna-sdk/blob/main/docs/ecossistema.md) no `lucerna-sdk`).

## Documentação

Veja a pasta [`docs/`](docs/):

- [`docs/ecossistema.md`](docs/ecossistema.md)

## Repositório

A branch `main` é protegida por um ruleset igual ao do `lucerna-shell`: PR obrigatório, conversas resolvidas, só merge commit, sem deleção nem force push. Os checks obrigatórios entram quando houver CI.

## Licença

[GPL-3.0](LICENSE).
