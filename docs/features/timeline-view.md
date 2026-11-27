# F008 — Zoom da timeline e snapping

## Objetivo e limites

Zoom é apresentação; snap é regra compartilhada de posicionamento.

## Contrato do core / biblioteca

Entrada pública: variante(s) `Command::SetTimelineZoom` / `Command::SetSnap` em `tessel_editor_core`, executada(s)
por `EditorState::execute`. Tipos auxiliares ficam em `tessel_editor_core::state`.
Consulte [o fluxo Rust](../core-architecture.md#usar-como-biblioteca) e as definições em
`crates/core/src/state/`. O estado e as validações são os mesmos para os adapters.
Resultados são JSON; falhas são `CommandError` com `code`, `message` e detalhes opcionais.
As pré-condições e limites são os implementados pelo modelo demo; não há transação de lote.

## CLI headless

```sh
./scripts/headless <<'JSONL'
{"op":"set","target":"timeline.snap","field":"value","value":true}
{"op":"status"}
JSONL
```

A resposta da operação vem em `result`. `status` é resumo, não snapshot completo do
inspetor; leia `EditorState::inspector` pela lib para consultas detalhadas de propriedades.

## UI e CLI da janela

O controle correspondente na biblioteca/inspetor/timeline traduz o gesto para o mesmo comando.
Na timeline, a roda vertical e a barra lateral percorrem as trilhas. Cabeçalhos e clipes
se deslocam juntos. O eixo horizontal percorre o tempo mantendo os cabeçalhos à esquerda.
A rolagem pertence à janela e não altera a sequência nem seu histórico.
Com a janela aberta:

```sh
./scripts/ui set timeline.snap value true
```

## Evidências e status

Regras cobertas por testes em `crates/core/src/state/timeline.rs`. Parser compartilhado: `crates/core/src/commands.rs`.
Ver [matriz](README.md) para distinguir o caminho implementado da validação específica.
Os testes GPUI do desktop não substituem a inspeção de cada interação na janela nativa.

BL169.5: teste GPUI de rolagem vertical sobre clipes e cabeçalhos, visibilidade
da última trilha, alinhamento e rolagem horizontal sem deslocar os cabeçalhos.
