# F053 — Cabeçalho do projeto aberto e miniaturas reais de splat/cena (BL144)

## Objetivo e limites

Duas lacunas do filme SHARP (BL144):

1. Depois de `project.open` pela CLI o cabeçalho seguia mostrando `North / Master`. Agora ele lê o estado vivo:
   `Projeto / Sequência` do projeto que está aberto, criado, restaurado ou renomeado, por qualquer superfície.
2. A biblioteca mostrava só o glifo de **splat** e de **cena**. Agora cada um tem uma miniatura **desenhada** pelo
   renderizador real (o mesmo do monitor e do export): um quadro da fonte do ativo, em `240 px` de largura e altura par
   na proporção da sequência.

Apresentação e cache descartável: nada disto é edição (sem revisão, sem undo, sem `dirty`). **Fora do escopo:** imagem
parada e vídeo (BL144.1, a operação `media.thumbnail` do FFmpeg não está ligada à biblioteca), invalidação por
reconstrução do runtime (BL144.3) e miniatura orientada pela captura do SHARP (BL144.2, depende da PR #23). O medidor e a
telemetria da PR #18 não foram tocados (`perf_meter.rs`, `status_bar.rs`, `perf.rs`); só a identidade/título do
cabeçalho mudou, sem alterar rótulos nem handlers de desempenho.

## Contrato do core / biblioteca

`core::header` (sem GPUI): `EditorState::header() -> Header { app, project, sequence, project_id, dirty, document }`,
`Header::label()` = `Projeto / Sequência` (só o projeto se a sequência for vazia) e `to_json()`.
`status.header` e `project.header` respondem o mesmo objeto; o nó semântico do título (`node:13`) e o texto pintado vêm de
`label()`. Nenhum estado paralelo: a janela lê o `EditorState` vivo a cada quadro.

`core::thumbnails` (sem GPUI nem renderer):

- **Quem recebe miniatura:** só `splat` e `scene` com arquivo e hash (`thumbnail_targets(retry)`).
- **Identidade** de uma miniatura: `v1|<tipo>|<hash do arquivo>|<largura>x<altura>`. Edições na linha do tempo não a
  invalidam; trocar o arquivo (hash) ou a proporção da sequência, sim. A miniatura só é mostrada
  (`EditorState::thumbnail`) enquanto a identidade guardada é a atual: um arquivo trocado volta a `pending`, nunca mostra a
  imagem antiga.
- **Carimbo (`Stamp`)** = ID do projeto + número da sessão (`ProjectSession`). Reabrir o mesmo documento é outra sessão.
  `set_thumbnail`/`set_thumbnail_failure` recusam com `stale_result` um resultado de outro carimbo ou de um ativo que mudou
  no meio do desenho, sem alterar nada. `prune_thumbnails` remove o que deixou de existir.
- **Falha** é lembrada por identidade (`code`, `message`); não é tentada de novo a cada tick, só com `retry: true`.
- `thumbnail_report(id)` → `{"state": "none|pending|ready|failed", ...}` (`path`, `width`, `height`, `identity` ou
  `code`/`message`). Aparece em `media:<id>` na árvore e em `media.status`.

`crates/media` (adaptador, injetado no `MediaHost`): `media.thumbnails {assets?, retry?}` e o agendamento automático da janela.

- Um job `thumbnail` (um por vez; reaproveitado se já houver um) desenha um quadro **por alvo** pelo renderizador do
  export, **fora da thread da UI** (`PreviewIntent::Source`, quadro 0). Falhar num alvo vira `Made::Failed` e não custa o
  quadro dos outros; só o cancelamento encerra o job e então **nada é aplicado**.
- O PNG vai para `<workspace>/.tessel/cache/<hash>/picture-<L>x<A>-<crc32>/thumb.png` (cache descartável, validado: tamanho
  do PNG e impressão `{cache, identity}`). Sem workspace o job espera por um (nada é escrito fora do projeto).
- **Splat avulso:** o runtime da `main` não desenha uma camada de splat sem cena. O job desenha-o como uma **cena de um
  objeto** (`splat.tessel-scene.json` + asset `thumb-<id>`) num **clone** do estado, nunca no projeto.
- **Identidade e cancelamento:** `poll` cancela o job cujo carimbo não é mais o do projeto aberto (`cancel_requested`; o
  job observa `ctx.check()` entre alvos) e só aplica resultado com carimbo atual. `finish` (CLIs) aplica e recusa o stale
  também; `job.commit` de um resultado de projeto anterior devolve `applied.stale: true`.
- O resultado volta pelo `poll` do tick da janela; a UI não espera por job algum.

Erros: `unknown_element` para ativo inexistente em `assets`; `unknown_element`/motivo (`reason`) se nada precisa de
miniatura; falha de renderização aparece em `thumbnail.failed` com o `code` real (ex.: runtime ausente), nunca escondida.

## CLI headless

`./scripts/render` (processo `tessel-render`, runtime real, **sem janela**; `./scripts/headless` não tem `MediaHost`):

```sh
./scripts/fixtures /tmp/fx
printf '%s\n' \
 '{"op":"project.create","name":"Aurora","sequence":"cut-a","width":320,"height":180,"length":48,"dir":"/tmp/aurora"}' \
 '{"op":"project.header"}' \
 '{"op":"media.import","paths":["/tmp/fx/cloud.splat","/tmp/fx/scene.tessel-scene.json"]}' \
 '{"op":"media.thumbnails"}' \
 '{"op":"media.status"}' | ./scripts/render
```

`project.header` → `{"label":"Aurora / cut-a","project":"Aurora","sequence":"cut-a",...}`; `media.thumbnails` (aguarda e
aplica) → `{"action":"media.thumbnails","applied":["cloud","scene-tessel-scene"],"failed":[],"stale":[]}`; `media.status`
traz `thumbnail.state: "ready"` e o `path` de cada PNG. Com `"wait":false` o job fica pendente e o resultado só vale em
`job.commit` se o projeto ainda for o mesmo. `./scripts/headless` responde `project.header` (sem mídia).

## UI e CLI da janela

- **Cabeçalho:** `node:13` (título) tem `label` = `Projeto / Sequência`; segue `project.create`, `project.open` e o
  restante sem ação de UI. `./scripts/ui get node:13`, `./scripts/ui op '{"op":"project.header"}'`.
- **Biblioteca:** o tile de `splat`/`scene` pinta a miniatura (`img`, `object-fit: cover`) quando a identidade bate, e o
  glifo antes disso; `./scripts/ui get media:<id>` mostra `state.thumbnail`. A janela agenda os jobs sozinha a cada tick
  (`schedule_thumbnails`) e `media.thumbnails` também funciona (responde com o job). Barra de status: "Miniaturas
  concluída/cancelada/falhou".
- **Correção vizinha (identidade):** a árvore de pastas só comparava IDs; todo projeto tem a mesma pasta raiz, então depois
  de abrir outro projeto a árvore seguia com o nome do anterior (`Aurora` dentro de `Basalt`, visto na captura anterior à
  correção). A assinatura passou a incluir o rótulo.
- Sem novo controle clicável: nenhum botão, atalho ou diálogo foi criado (por isso nenhum `disabled_reason` nem
  `Workflow::has_dialog`). As operações novas são `project.header` e `media.thumbnails`.

## Evidências e status

| Superfície | Evidência | Resultado |
|---|---|---|
| Lib (core) | `crates/core/tests/thumbnails.rs` (7) + 2 unitários de `header` + 3 de `thumbnails` | passam; positivos e negativos (outro projeto, outra sessão, hash trocado, falha lembrada/retry) |
| Lib (media) | `crates/media/tests/thumbnails.rs` (8) com renderer duplo | passam: um quadro por alvo, cache sem renderer, falha isolada, janela sem laço, projeto sem workspace espera, job cancelado e **nunca aplicado ao próximo**, `job.commit` stale |
| Runtime real | `crates/render/tests/thumbnail_frames.rs` (1), `TESSEL_REQUIRE_RUNTIME=1` | passa: cena e splat avulso desenhados em Chrome/Three.js, sondas de pixel contra um pinhole calculado no teste |
| UI (GPUI) | `workstation/identity_tests.rs` (2) + `library_panel::tests` (1) | passam: o cabeçalho segue o projeto (largura medida do título), o tile pinta a imagem só enquanto ela é a do ativo; a árvore muda de rótulo com a mesma pasta |
| Headless | `scripts/header-thumbnails-proof.py` (`tessel-render`) | 53/53 checks |
| Janela nativa | mesmo script, binário direto com `HOME` próprio, só `ui op/get` | 56/56 checks; capturas `assets/bl144-header-thumbnails-window.png` (A) e `...-window-b.png` (B, depois de reabrir) |

Rodar: `python3 scripts/directed-scene-proof/with-gpu-lock.py python3 scripts/header-thumbnails-proof.py`.

O que o script prova nas duas superfícies: título `Aurora / cut-a` e `Basalt / cut-b` após create/open (e `node:13`
na janela); miniaturas de splat e cena em `240x136` dentro do cache **do workspace do próprio projeto**; PNG lido por um
decodificador independente e pixels conferidos num pinhole calculado no próprio script (caixa laranja e esfera azul nos pontos
projetados, nuvem no centro, **splat sem caixa nem esfera**, cantos de fundo); desenhar não é edição (revisão, `dirty` e
histórico iguais); ativo desconhecido recusado sem tocar o projeto. Os dois projetos **têm os mesmos IDs e arquivos** de
propósito: o job de B nunca vira miniatura de A. Só no headless: `job.commit` de B com A aberto → `stale`. Só na janela: o
job de B que estava rodando quando A o substituiu terminou `cancelled` (observado: `['cancelled','succeeded','succeeded']`).

**Limites declarados.**

- O monitor aparece **preto** nas duas capturas (a WebView do monitor não desenhou o quadro naquele instante); o que as
  capturas mostram é o cabeçalho, a biblioteca e os nomes. O monitor não é evidência desta feature.
- SwiftShader, não GPU real; sem ponteiro físico (não há controle novo para clicar).
- Imagem parada e vídeo seguem com glifo (BL144.1). A identidade não contém a captura do SHARP e o splat do SHARP seria
  desenhado sem a orientação dela (BL144.2, após a PR #23). O cache não é invalidado por uma reconstrução do runtime, só por
  `VERSION`/`CACHE_VERSION` (BL144.3).
- Mutações executadas e restauradas (prova de que os testes discriminam): aplicar sem conferir o carimbo, mostrar sem conferir a
  identidade, título fixo no cabeçalho e assinatura da árvore sem o rótulo.
