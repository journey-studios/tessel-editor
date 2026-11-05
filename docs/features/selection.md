# F009 — Seleção de tracks e clipes

## Objetivo e limites

Sincroniza monitor e inspetor no modelo; sem renderização de frames de vídeo.

## Contrato do core / biblioteca

Entrada pública: variante(s) `Command::SelectTrack` / `Command::SelectClip` em `tessel_editor_core`, executada(s)
por `EditorState::execute`. Tipos auxiliares ficam em `tessel_editor_core::state`.
Consulte [o fluxo Rust](../core-architecture.md#usar-como-biblioteca) e as definições em
`crates/core/src/state/`. O estado e as validações são os mesmos para os adapters.
Resultados são JSON; falhas são `CommandError` com `code`, `message` e detalhes opcionais.
As pré-condições e limites são os implementados pelo modelo demo; não há transação de lote.

## CLI headless

```sh
./scripts/headless <<'JSONL'
{"op":"invoke","target":"clip:atlantic-coast","action":"select"}
{"op":"status"}
JSONL
```

A resposta da operação vem em `result`. `status` é resumo, não snapshot completo do
inspetor; leia `EditorState::inspector` pela lib para consultas detalhadas de propriedades.

## UI e CLI da janela

O controle correspondente na biblioteca/inspetor/timeline traduz o gesto para o mesmo comando.
Com a janela aberta:

```sh
./scripts/ui invoke clip:atlantic-coast select
```

## Evidências e status

Regras cobertas por testes em `crates/core/src/state/mod.rs` e `crates/core/src/state/timeline.rs`. Parser compartilhado: `crates/core/src/commands.rs`.
Ver [matriz](README.md) para distinguir o caminho implementado da validação específica.
Os testes GPUI do desktop não substituem a inspeção de cada interação na janela nativa.
