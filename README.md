<p align="center"><img src="docs/assets/logo.svg" alt="Lucerna" height="120"></p>

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

## Desenvolvimento

O CI (`fmt`, `clippy` e `test`, todos obrigatórios no `main`) roda estes comandos; rode-os antes de abrir o PR:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

Os testes de processo criam sockets Unix, então o `TMPDIR` precisa ser curto (o CI fixa `TMPDIR=/tmp`). O teste que usa o `systemctl --user` de verdade é `#[ignore]` e só roda numa sessão com systemd de usuário:

```bash
cargo test --workspace --locked -- --ignored
```

## Licença

[GPL-3.0](LICENSE).
