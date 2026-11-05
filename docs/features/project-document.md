# F022 — Documento do projeto: salvar, abrir, autosave e recuperação (P01/P02)

## Status

Lib **V** (`crates/core/tests/project.rs`, `proof.rs`, testes de `persist`/`document`), CLI headless e
`scripts/media` **V**, CLI da janela **V** (`ui op`), UI **V** (diálogos Abrir / Salvar como / Recuperar, botão
Salvar, estado "salvo/não salvo" na barra; `scripts/live-flow-proof.py`).

## Documento

`<nome>.tessel.json`, UTF-8, serialização **canônica** (chaves ordenadas): salvar duas vezes um projeto
inalterado produz os mesmos bytes. Limite de leitura: 256 MiB (`document_too_large`).

```text
format "tessel-project", schema_version 1
project      { id, name, revision, requires { spark "2.4.0", three "0.187.0" } }
library      { folders[], assets[] }       identidade (id) e conteúdo (hash) separados
sequences[]  { id, name, rate {num,den}, length, width, height, tracks[], clips[] }
active_sequence
export       { preset, user_presets[] }    presets do usuário vivem no documento
presentation { playhead, zoom, snap, ... } opcional; um loader pode ignorar
```

Caminhos de mídia gerenciada são **relativos** ao documento (`/` como separador); só caminhos dentro da
pasta são relativizados. IDs são strings (o interner não é serializado); tempo e FPS são racionais.
Versão de schema desconhecida → `unsupported_schema`; documento malformado → `invalid_document` /
`invalid_json`. Um erro **nunca** altera a sessão aberta: o estado candidato é construído antes do swap.

## Operações (`ops::parse_op`, iguais em lib, headless e janela)

| `op` | Campos | Resultado / erros |
|---|---|---|
| `project.save` | `path?` (omitido: o documento atual) | arquivo temporário + rename atômico; define o workspace (pasta do documento); descarta recuperações; `invalid_path`, `write_failed`, `no_document` |
| `project.open` | `path` | `document_unreadable`, `invalid_json`, `invalid_document`, `unsupported_schema`, `document_too_large`; mídia fora do lugar abre como `missing` (relink) |
| `project.autosave` | — | `.tessel/recovery/` ao lado do documento; `changed:false` com projeto limpo; mantém 5 revisões |
| `project.recover` | `path`, `revision?`, `apply?` (padrão `true`) | `apply:false` só lista; `no_recovery` |
| `project.discard_recovery` | `path` | remove as revisões autosalvas |
| `project.describe` | `path` | lê sem abrir: nome, id, sequências, nº de assets, recuperações |

`status`/`summary` traz `persistence {document, dirty, revision, saved_revision}`. Um projeto recuperado fica
**sujo** (nunca foi salvo). Autosave não substitui o save explícito: as revisões vivem numa área separada e o
documento só muda em `project.save`.

## UI

Header: Abrir (`node:18`), Salvar (`node:19`; abre "Salvar como" se não há documento), Desfazer/Refazer.
Diálogos `flow.open`, `flow.save_as`, `flow.recover` com seletor nativo de arquivos opcional. A janela autosalva
a cada 20 s quando há alterações (`workflow_tick`) e oferece recuperar. IDs e exemplos:
[ui-automation](../ui-automation.md).

## Limites

`presentation` é preservada mas não afeta render. Só existe schema v1 (sem migrações). O desktop não impede
fechar a janela com alterações não salvas: o autosave é a rede de segurança, não um aviso.

## Projetos com cenas dirigidas

F036 adiciona o bloco `scenes` e escreve schema 2 somente quando esse bloco é necessário.
Projetos comuns permanecem em schema 1. Leitores antigos recusam schema 2, protegendo as
cenas contra perda silenciosa. Save/autosave e pacote conferem os bytes serializados reais
contra o limite do loader (256 MiB), antes de escrever ou substituir o arquivo.
