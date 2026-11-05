# F005 — Sliders, selects e switches de propriedades

## Objetivo e limites

Valores por clipe selecionado, com clamp/opções/tipo/aplicabilidade. Transformações e animação são estado demonstrativo, sem processamento real.

## Contrato do core / biblioteca

Entrada pública: variante(s) `Command::SetProperty` em `tessel_editor_core`, executada(s)
por `EditorState::execute`. Tipos auxiliares ficam em `tessel_editor_core::state`.
Consulte [o fluxo Rust](../core-architecture.md#usar-como-biblioteca) e as definições em
`crates/core/src/state/`. O estado e as validações são os mesmos para os adapters.
Resultados são JSON; falhas são `CommandError` com `code`, `message` e detalhes opcionais.
As pré-condições e limites são os implementados pelo modelo demo; não há transação de lote.

## Gesto de slider (BL062): preview ao vivo, uma entrada no undo

Um arrasto de slider **não** vira uma entrada de undo por tick. O ciclo é um comando do core
(`Command::PropertyGesture`), reutilizado por UI, CLI e biblioteca (`crates/core/src/state/property_gesture.rs`):

| Fase | Efeito |
|---|---|
| `begin` | Fixa clipe e propriedade, guarda o valor original e um snapshot. Valida como `SetProperty` (dry-run) e liquida qualquer gesto anterior |
| `update` | Aplica o valor **na hora** (preview visível, mesmo clamp/validação/erros de `SetProperty`). Não grava nada e não apaga o redo |
| `commit` | **Uma** entrada "Set property" do valor original ao final. Sem entrada se nada mudou ou se voltou ao original |
| `cancel` | Restaura o original, sem entrada |

Política segura: o gesto fica no clipe do `begin` mesmo se a seleção mudar; qualquer outra edição,
undo, redo, lote, troca de sequência ou novo `begin` o **confirma** antes de rodar; mudança do documento
por baixo (revisão, sequência, clipe) o descarta com erro tipado (`stale_gesture`); presentation (zoom da
timeline, seleção, abas) não o encerra. A confirmação respeita `transaction` (dentro dela o lote é a entrada).
`SetProperty` discreto continua uma entrada por chamada.

Sliders cobertos: todos os `PropertyKind::Slider` de `PROPERTIES` (persistidos, entram no histórico). O zoom
da **timeline** é apresentação: nunca entra no histórico.

## CLI headless

```sh
./scripts/headless <<'JSONL'
{"op":"set","target":"property:zoom","field":"value","value":150}
{"op":"status"}
JSONL
```

Gesto (preview ao vivo, uma entrada ao confirmar):

```sh
./scripts/headless <<'JSONL'
{"op":"property.gesture.begin","property":"zoom"}
{"op":"property.gesture.update","value":120}
{"op":"property.gesture.update","value":150}
{"op":"property.gesture.commit"}
JSONL
```

`clip` (opcional, padrão: o clipe selecionado) e `gesture` (opcional, padrão: o aberto) podem ser passados;
`property.gesture.cancel` desfaz o preview. O estado aberto aparece em `history.status` (`history.gesture`).

A resposta da operação vem em `result`. `status` é resumo, não snapshot completo do
inspetor; leia `EditorState::inspector` pela lib para consultas detalhadas de propriedades.

## UI e CLI da janela

O controle correspondente na biblioteca/inspetor/timeline traduz o gesto para o mesmo comando.
Com a janela aberta:

```sh
./scripts/ui set property:zoom value 150
```

## Evidências e status

Regras cobertas por testes em `crates/core/src/state/inspector.rs`. Parser compartilhado: `crates/core/src/commands.rs`.
Ver [matriz](README.md) para distinguir o caminho implementado da validação específica.
Os testes GPUI do desktop não substituem a inspeção de cada interação na janela nativa.

Gesto: testes do core em `crates/core/tests/slider_gesture.rs`; entrada GPUI real (press/move/release no slider
do Kit, Escape, perda de foco, teclado) em `crates/desktop/src/workstation/slider_gesture_tests.rs`. Detalhe,
limites e o que foi (ou não) executado em [slider-gesture-history-report.md](../slider-gesture-history-report.md).
Limite conhecido: as ações de acessibilidade Aumentar/Diminuir do slider do Kit não emitem evento e não chegam ao modelo.

Evidência da integração BL062: 28 testes core, 13 testes de input GPUI e sessão do executável headless passaram; clippy core/desktop limpos. O gate completo da rodada de migração ainda aguarda recursos; o socket dos sliders já foi provado no release atualizado. Input GPUI não prova mouse físico (BL043).

Release e prova socket concluídos: [slider-live-proof](../slider-gesture-live-proof.json), 12/12 checagens em janela nativa isolada, sem ponteiro físico. O app do usuário foi atualizado preservando snapshot local.
