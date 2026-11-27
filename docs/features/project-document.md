# F022 — Documento do projeto: salvar, abrir, autosave e recuperação (P01/P02)

## Status

Lib **V** (`crates/core/tests/project.rs`, `proof.rs`, testes de `persist`/`document`), CLI headless e
`scripts/media` **V**, CLI da janela **V** (`ui op`), UI **V** (diálogos Abrir / Salvar como / Recuperar, botão
Salvar, estado "salvo/não salvo" na barra; `scripts/live-flow-proof.py`).

## Documento

`<nome>.tessel.json`, UTF-8, serialização **canônica** (chaves ordenadas): salvar duas vezes um projeto
inalterado produz os mesmos bytes. Limite de leitura: 256 MiB (`document_too_large`).

```text
format "tessel-project", schema_version 1 a 5 conforme os recursos usados
project      { id, name, revision, requires { spark "2.4.0", three "0.187.0" } }
library      { folders[], assets[] }       identidade (id) e conteúdo (hash) separados
sequences[]  { id, name, rate {num,den}, length, width, height, tracks[], clips[] }
active_sequence
scenes       { version, items[], ... }     opcional, exige schema_version 2 ou superior
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

`presentation` é preservada e afeta apenas a interface. O documento é salvo no menor esquema
capaz de preservar os recursos do projeto. Ao fechar a janela com alterações pendentes,
o mecanismo de recuperação é o autosave.

## Projetos com cenas dirigidas

F036 adiciona o bloco `scenes` no schema 2. F075 acrescenta `camera_push_in` ao clipe
no [schema 3](camera-push-in.md). Um leitor antigo recusa um esquema mais novo para
proteger os dados que não consegue ler. Save/autosave e pacote conferem os bytes serializados reais
contra o limite do loader (256 MiB), antes de escrever ou substituir o arquivo.

## Projetos localizados

Projetos com idiomas habilitados usam o schema 4 e gravam o bloco `localization`
com ajustes e aprovações por idioma. A composição-base continua no bloco
`sequences`. Veja [F078](localization.md).

## Janelas de fade herdadas (BL081.2)

O campo opcional `fade_window` em `clips[]` exige schema 5 em qualquer projeto.
A janela preserva as rampas após um corte. As partes `lead` e `tail` guardam
a âncora em quadros e o valor de fade da rampa. Em clipes de áudio, `norm` é
obrigatório e guarda a normalização congelada. Clipes visuais recusam `norm`.
O leitor recusa uma janela gravada em schemas 1–4, que não permitem preservar
esse campo em versões anteriores do editor.
