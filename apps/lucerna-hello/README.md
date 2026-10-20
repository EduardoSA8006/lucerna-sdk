# lucerna-hello

O app de exemplo do Lucerna: um contador. É o molde que os apps copiam.

## Como é feito

- **lib + bin.** `src/lib.rs` monta tudo e é o que os testes usam: `hello()`
  devolve o `ServiceBuilder` com os tópicos e os métodos, e `cli()` devolve a
  linha de comando (`lucerna_cli::CliApp`). `src/main.rs` só escolhe entre
  `lucerna-hello serve` (o serviço) e os subcomandos.
- **Tópicos.** `counter` (`{"value": n}`) e o contínuo `ticks`
  (`{"tick": k, "bands": [64 × f32]}` a cada 1 ms, produzido só com assinante).
- **Métodos.** `counter.add {"n"}`, `counter.reset` e `quiet.set {"on"}`.
- **Shell.** Com o contador diferente de zero, o item `counter` fica na barra
  com o valor; `quiet.set` pede ou devolve o não perturbe. Sem shell, nada dá
  erro: o estado é reaplicado quando ele volta.
- **Testes.** `tests/protocol.rs` fala o protocolo com o serviço em processo,
  num socket temporário; `tests/shell.rs` usa o `FakeShell` do
  `lucerna-service`; `tests/files.rs` confere o manifesto e as unidades.

## Linha de comando

```sh
lucerna-hello add 3        # Contador: 3
lucerna-hello reset        # Contador: 0
lucerna-hello quiet on     # liga o não perturbe
lucerna-hello status       # estado dos tópicos
lucerna-hello watch counter
```

## Instalação

O binário vai para `/usr/bin/lucerna-hello`; as unidades e o manifesto, para
as pastas do systemd do usuário e do Lucerna:

```sh
sudo install -Dm755 target/release/lucerna-hello /usr/bin/lucerna-hello
sudo install -Dm644 systemd/lucerna-hello.socket systemd/lucerna-hello.service \
    -t /usr/lib/systemd/user/
sudo install -Dm644 data/hello.json /usr/share/lucerna/apps/hello.json
systemctl --user daemon-reload
systemctl --user enable --now lucerna-hello.socket
```

Num pacote, troque `/usr` pelo prefixo do empacotador (ex.: `$pkgdir/usr`) e
tire o `sudo`; o `ExecStart=` da unidade aponta para `/usr/bin`.

O systemd escuta em `$XDG_RUNTIME_DIR/lucerna/hello.sock` (pasta `0700`,
socket `0600`) e sobe o serviço na primeira conexão; o serviço sai sozinho
depois de 60 s sem clientes e sem itens na barra.

Para desenvolver sem instalar, `cargo run -p lucerna-hello -- serve` cria o
socket à mão no mesmo lugar (`LUCERNA_IDLE_TIMEOUT_MS` encurta a ociosidade).
O manifesto do usuário, em `$XDG_DATA_HOME/lucerna/apps/hello.json`, vence o
do sistema.
