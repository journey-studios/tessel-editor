# Triagem dos docs do piloto: o que levar para o `tessel-editor` (2026-11-11)

Pergunta: dos documentos `.md` do piloto do Scene Studio (`t3/scene-engine-pilot`, commit
`d40ac3f7c0be4feccc54103e7619bbe7301dc325`), quais precisam viver aqui, já que este repositório é a nova engine?

**Resultado:** 52 documentos avaliados. **10 foram trazidos** como cópia literal em
[`origin-specs/`](origin-specs/README.md). Os outros 42 ficam de fora, e a tabela diz por quê, um a um.
Os 49 `.md` dos pacotes de vídeo (`output/video-production`) também foram classificados; nenhum entra no repositório.

## Por que agora

O inventário MIG leu o piloto "apenas em nível de catálogo" ([README](README.md)): de `docs/video-production/*`
ele registra "títulos/estado" e anota que `scene-effects` e `scene-element-recipes` não foram lidas. Ao mesmo tempo,
[pilot-fx-shots-studio-features.md](pilot-fx-shots-studio-features.md) usa `scene-fx-plan.md`,
`scene-fx-decisions.md`, `scene-shot-trim-plan.md`, `scene-shot-trim-decisions.md` e `studio-core-boundary.md` como
**fonte**. Essas fontes estavam num commit que **não existe em nenhum remoto da origem**: o branch remoto parava em
`3655790767`, e o local, em `d40ac3f7c0`, tinha 24 commits a mais.

## Método e limites

- Universo: os `.md` do piloto ligados a cena (`docs/video-production/**`, `roadmaps/video-production-pipeline.md`,
  `packages/scene-engine/**`, `apps/scene-studio/**` e `.claude/skills/scene-*`): 52 arquivos.
- Para cada um li o título, o estado e o primeiro parágrafo, e cruzei com este repositório: quais docs daqui o citam
  (contagem por nome de arquivo) e o que a matriz [feature-migration-matrix.md](feature-migration-matrix.md) e a
  [tabela de features](../features/README.md) já registram.
- **Não li por inteiro** os 52 documentos. Os 10 trazidos foram varridos por termos de conteúdo privado, credenciais e
  caminhos pessoais (nada encontrado) e comparados byte a byte com a origem, mas a leitura integral fica para quem for
  portar cada frente.
- Cobertura medida no `origin/main` de 2026-11-11 (`9fc491b`); a tabela de features diz, por exemplo, que o F016 cobre
  3 dos 59 efeitos.

## Critérios

| Decisão | Quando | Destino |
|---|---|---|
| **Trazido** | Descreve comportamento que o Rust ainda precisa reproduzir, ou é decisão/plano que outro doc daqui cita | Cópia literal em `docs/migration/origin-specs/` |
| **Recriar depois** | Ensina a operar a CLI antiga (`scene`); só serve com paridade da CLI headless | Backlog BL162.2 |
| **Fora de escopo** | Pipeline de assets por IA, scripts Python de reconstrução ou roadmap de produto: a matriz marca "não migrar" (MIG107) e o `AGENTS.md` proíbe portar o motor antigo inteiro | Só no backup da origem |
| **Histórico** | Plano já executado, relatório de entrega ou revisão: o resultado está no código ou nos docs trazidos | Só no backup da origem |
| **Obsoleto** | Descreve peça da origem sem equivalente aqui | Descartar |
| **Privado** | Contém conteúdo de campanha | Nunca no repositório (ver [`projects/README.md`](../../projects/README.md)) |

## Trazidos (10)

| Arquivo | Linhas | O que é | Por quê |
|---|---:|---|---|
| `CONTEXT.md` | 93 | Glossário do domínio de produção audiovisual | Nenhum doc daqui define esses termos num só lugar |
| `camera-commands.md` | 158 | Os 15 comandos de câmera (movimento, velocidade, tempo) | MIG033 registra que nenhum dos 15 existia; o F054 cobre keyframes por clipe, outra superfície |
| `scene-effects.md` | 262 | Catálogo dos "looks" de pós-produção | F016 cobre 3 dos 59; a migração não o leu por inteiro |
| `scene-fx-plan.md` | 1073 | Plano (revisão 3) da camada de efeitos | As decisões citam as seções dele; é fonte de `pilot-fx-shots-studio-features.md` |
| `scene-fx-decisions.md` | 254 | Decisões #1 a #134 e tratamento dos achados | Fonte de `pilot-fx-shots-studio-features.md` |
| `scene-shot-trim-plan.md` | 120 | Plano de aparar, alongar e mover o corte dos planos | MIG102: ripple/roll e propagação para fx, chaves e narração não existem aqui |
| `scene-shot-trim-decisions.md` | 223 | Decisões sobre a política de ripple e desvios | idem |
| `studio-core-boundary.md` | 117 | Fronteira Studio ↔ core | Mesma regra do `AGENTS.md`: lib, CLI e UI consomem as regras do core |
| `studio-properties.md` | 129 | Tudo que o inspector edita numa cena | Checklist de paridade do inspector |
| `scene-engine-api.md` | 165 | API do piloto: biblioteca, módulo Node, CLI `scene` | Referência de paridade para `tessel-headless`; voz/TTS fica fora (MIG107) |

## Recriar depois (3)

| Arquivo | Linhas | O que é | Por quê |
|---|---:|---|---|
| `.claude/skills/scene-cli/SKILL.md` | 395 | Skill que ensina um agente a operar a CLI `scene` | Os comandos são os da CLI antiga; recriar sobre `tessel-headless` quando houver paridade |
| `.claude/skills/scene-effects/SKILL.md` | 78 | Skill de efeitos | idem |
| `.claude/skills/scene-element-recipes/SKILL.md` | 310 | Receitas de elementos de cena (inclui duração dos planos e canais independentes de imagem e áudio) | idem; a migração registra que não foi lida |

## Fora de escopo (11)

| Arquivo | Linhas | O que é | Por quê |
|---|---:|---|---|
| `docs/video-production/ai-asset-production-spec.md` | 276 | Proposta de produção e organização de assets por IA | MIG107; o próprio doc diz depender de prova futura |
| `docs/video-production/ai-asset-production-references.md` | 87 | Pesquisa de referências para preparar assets por IA | MIG107 |
| `docs/video-production/scene-pattern-catalog.md` | 233 | Catálogo de padrões/peças da pipeline de assets | MIG107; "proposta documental" |
| `docs/video-production/threejs-planar-video-references.md` | 151 | Pesquisa de tutoriais e projetos externos | Referência de estudo, não especificação |
| `docs/video-production/reviews/ai-assets-opus55-initial-review.md` | 645 | Revisão inicial sobre assets por IA | MIG107 |
| `docs/video-production/reviews/ai-assets-opus55-discussion.md` | 185 | Resposta aos contrapontos da revisão | MIG107 |
| `roadmaps/video-production-pipeline.md` | 689 | Roadmap do produto "produção automática de vídeos narrados" | Roadmap do produto de origem, não da engine |
| `packages/scene-engine/scripts/elements/rig/README.md` | 52 | Protótipo da "mão que escreve" (rig por IK) em Python/Three.js | Script de laboratório |
| `packages/scene-engine/scripts/scene-pipeline/README.md` | 80 | Imagem → cena splat com Apple SHARP | O Rust importa o PLY do SHARP ([F052](../features/sharp-import.md)); o script que o gera não será portado |
| `packages/scene-engine/scripts/world-model/README.md` | 78 | Testes de world-model em RunPod | Experimento de pipeline |
| `packages/scene-engine/scripts/world-model/viewer/README.md` | 10 | Como ver um mundo gerado | Experimento de pipeline |

## Histórico (27)

| Arquivo | Linhas | O que é | Por quê |
|---|---:|---|---|
| `docs/video-production/scene-effects-plan.md` | 430 | Plano do catálogo, da CLI e da doc de efeitos; estado "done" (2026-11-02) | Executado; `scene-effects.md` descreve o resultado |
| `docs/video-production/scene-fx-s1-report.md` | 182 | Relatório do spike S1 (dois planos no mesmo quadro), com medições | Medições do piloto em WebGL; o Rust mede o próprio custo |
| `docs/video-production/scene-engine-api-plan.md` | 315 | Plano da API única (biblioteca, módulo, CLI, preview fino); estado "planned" | `scene-engine-api.md` descreve o resultado |
| `docs/video-production/scene-engine-spec.md` | 437 | Proposta consolidada de 30/10 da engine de cenas | Anterior ao código; o inventário MIG parte do código |
| `docs/video-production/scene-engine-index.md` | 38 | Índice da direção de 30/10 (peças e poses, pipeline de assets) | Direção de produto, superada |
| `docs/video-production/scene-engine-learning-log.md` | 48 | Aprendizados da preparação de assets | Ligado à pipeline de assets (MIG107) |
| `docs/video-production/scene-studio-plan.md` | 174 | Plano de transporte com áudio, inspeção e plugins | Parte executada; o resto é plano do Studio web |
| `docs/video-production/studio-properties-plan.md` | 39 | Plano em três passos das propriedades | Executado; `studio-properties.md` descreve o resultado |
| `docs/video-production/studio-rig-projects-plan.md` | 302 | Plano de vários projetos, bones e linha de follow | **Desatualizado na origem**: diz "sem código no editor", mas o inventário MIG registra rig, follow e projetos múltiplos no código da origem |
| `docs/video-production/normal-maps.md` | 67 | Normal maps para personagens | O Rust já trata normal maps (citados em 6 docs e 14 arquivos Rust daqui) |
| `docs/video-production/deliveries/*.md` (14) | 23 a 78 | Notas de entrega por marco: primeira entrega e revisão do piloto, export MP4 de cena splat, luz (3), clima (2), splat do SHARP, pipeline imagem → splat, transporte (entrega e revisão), modelo de mundo | Registro de entregas já feitas; `pilot-fx-shots-studio-features.md` e o inventário cobrem o conteúdo |
| `packages/scene-engine/README.md` | 131 | Visão geral do pacote do piloto (VGP-12) | Coberta pelo inventário MIG |
| `packages/scene-engine/src/fx/__baseline__/README.md` | 65 | Baseline de pixels do piloto: bundle sem `fx` deve renderizar igual | O Rust usa o [oráculo de migração](../features/migration-oracle.md) |
| `packages/scene-engine/src/runtime/fx/README.md` | 89 | Compositor de efeitos Three.js do piloto | Descreve o runtime de origem; o runtime daqui é `runtime/` |

## Obsoleto (1)

| Arquivo | Linhas | O que é | Por quê |
|---|---:|---|---|
| `apps/scene-studio/src/ui/leva/README.md` | 17 | Nota sobre um helper React/Leva do Studio web | A UI deste repositório é GPUI |

## Pacotes de vídeo (`output/video-production`, 49 `.md`)

Fora do git da origem. Nenhum deles entra neste repositório.

| Pacote | `.md` | Decisão |
|---|---:|---|
| Dois pacotes do filme da campanha (o de 120 s e o piloto) | 31 | **Privado.** Os próprios READMEs dizem que contêm conteúdo privado da campanha. Ficam fora do repositório; o filme vira projeto local em `projects/` (ignorado pelo Git) |
| `scene-reconstruction-benchmark-*` | 12 | Fora de escopo: relatórios de benchmark de reconstrução (MoGe, DA3, TripoSplat) |
| `gaussian-splat-experiment-*` | 3 | Fora de escopo: experimento de Gaussian Splat |
| `scene-engine-pilot-01-review-*` | 3 | Histórico: revisões visuais da primeira entrega |

Dentro dos pacotes privados há duas peças **genéricas** que valem uma extração futura, depois de revisão do dono:
os 16 tipos de plano de câmera (`specs/scene-types.md`) e a skill de animação de cenas em camadas (`skill/`). Pendência
BL162.3.

## Achados

1. **24 commits só locais.** O piloto tinha commits que não existiam em nenhum remoto, e há outros 157 branches com
   trabalho só local no repositório de origem. Foram preservados num espelho completo antes de qualquer limpeza.
2. **Decisões e plano são uma unidade.** `scene-fx-decisions.md` e `scene-shot-trim-decisions.md` citam seções
   (`§4.2`, `§8.1`, `§5`) dos planos; por isso os planos de efeitos e de duração dos planos vieram junto.
3. **O vocabulário de 15 comandos de câmera não existe aqui.** Não há `CameraCommand` nem `camera.<op>` em
   `crates/core/src`; o F054 entrega keyframes por clipe, que é outra coisa.
4. **Um doc de origem está desatualizado** (`studio-rig-projects-plan.md`), o que o README da migração já registrava.

## Pendências

Registradas no [backlog](../backlog.md#bl162--triagem-dos-docs-do-piloto-e-projetos-locais): BL162.1 (comandos de
câmera), BL162.2 (skills), BL162.3 (tipos de plano), BL162.4 (convenção `projects/`).

## Como reabrir um doc que ficou de fora

O espelho completo do repositório de origem está no backup do mantenedor (2026-11-11). Com ele, qualquer arquivo sai
do commit original sem restaurar nada:

```sh
git --git-dir=<espelho>/tesselapp.git show d40ac3f7c0:docs/video-production/scene-engine-spec.md
```
