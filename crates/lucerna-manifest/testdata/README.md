# Exemplos compartilhados

`manifest-cases.json` reúne os casos de compatibilidade da `api` do painel e de manifestos válidos e inválidos (com o `kind` do erro esperado).
Quem lê: os testes do `lucerna-manifest` (`tests/manifest.rs`) e os testes do shell (`lucerna-shell`).
O shell testa contra este mesmo arquivo, para que os dois lados concordem com o contrato. Ele fica dentro do crate para ir junto no pacote publicado no crates.io; o caminho no repositório é `crates/lucerna-manifest/testdata/manifest-cases.json`.
