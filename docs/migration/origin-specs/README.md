# Especificações da origem (somente leitura)

Cópia **literal** de dez documentos do piloto do Scene Studio: o branch `t3/scene-engine-pilot` do repositório de
origem, commit `d40ac3f7c0be4feccc54103e7619bbe7301dc325` (2026-11-04), pasta `docs/video-production/`. Foram
escolhidos na triagem [pilot-docs-triage-2026-11-11.md](../pilot-docs-triage-2026-11-11.md): descrevem
comportamento que o Rust ainda precisa reproduzir, ou são decisões que outros documentos daqui citam.

## Regras de uso

- **Não são contrato do Rust.** O contrato e as evidências vivem em [`docs/features/`](../../features/README.md);
  a classificação do que existe na origem vive no [inventário MIG](../README.md). Estes arquivos só explicam o
  comportamento de origem.
- **Não edite estes arquivos.** Se um deles divergir do que o Rust faz, registre a divergência no doc de
  feature ou na triagem e aponte para cá. Os hashes abaixo permitem conferir que a cópia continua literal.
- Caminhos como `src/…`, `bin/…` e `apps/scene-studio/…`, e os identificadores de ticket (`JOU-`, `VGP-`)
  pertencem à origem e não existem neste repositório. Links para documentos da origem que não foram trazidos
  (hoje: `scene-effects-plan.md`, `scene-engine-api-plan.md` e `scene-fx-s1-report.md`, citados em cinco
  links) ficam sem destino de propósito: ver a triagem, que diz onde cada um está classificado e como reabri-lo.
- A tabela da seção 3 de `studio-core-boundary.md` traz `|` sem escape dentro de comandos da CLI (linhas 58, 63 e
  64). Ao renderizar, esses `|` abrem colunas a mais e a linha sai desalinhada. É do original e a cópia continua
  literal; a versão corrigida dessas três linhas, com `\|`, está logo abaixo.
- O commit `d40ac3f7c0` **não está em nenhum remoto da origem** (o branch remoto parava em `3655790767`). Estas
  cópias existem para que a especificação não dependa de um commit que só existia numa máquina.

| Gesto no Studio | Comando do core | CLI (`scene …`, JSON, saída 0/1/2, nada gravado sem `--write`) |
| --- | --- | --- |
| arrastar borda/corte de plano | `trimShot`, `moveCut`, `setShotDuration`, `setEditDuration` | `shots <bundle> trim\|move-cut\|duration\|edit-duration` |
| mover/escalar chaves de progresso de um follow | `retimeFollowKeys`, `setFollowProgress` (rig) | `follow <bundle> <rigId> <followId> move-key\|move-all\|scale …` |
| abrir o painel de um clipe/transição/plano | `describeFxInspector`, `describeShotDetail` | `inspect <bundle> clip\|transition\|shot <id> [--frame N]` |

## Arquivos

| Arquivo | Linhas | sha256 (16) | Por que está aqui | Onde o Rust está hoje |
|---|---:|---|---|---|
| [CONTEXT.md](CONTEXT.md) | 93 | `b2c69b22c78b6cd9` | Glossário do domínio de produção (cena, plano, instância, peça…); nenhum doc daqui define esses termos num só lugar | Vocabulário, sem porte |
| [camera-commands.md](camera-commands.md) | 158 | `7f2d6cbc00a531cd` | Os 15 comandos de câmera do piloto (movimento, velocidade e tempo), com erros tipados e dica | MIG033 registra que nenhum dos 15 existia; [F054](../../features/camera-keys.md) cobre keyframes por clipe, outra superfície. Pendente: BL162.1 |
| [scene-effects.md](scene-effects.md) | 262 | `ec99d9ebf78b6c63` | Catálogo dos "looks" (efeitos de pós-produção do quadro); a migração registra que este doc não foi lido por inteiro | [F016](../../features/effects.md) cobre vinheta, fade e `filmGrain`: 3 dos 59 efeitos (BL082) |
| [scene-fx-plan.md](scene-fx-plan.md) | 1073 | `8dafdf3d71393193` | Plano (revisão 3) da camada de efeitos; as decisões citam as seções dele (§4.2, §8.1…) | idem |
| [scene-fx-decisions.md](scene-fx-decisions.md) | 254 | `11bf4c08263082d1` | Decisões #1 a #134 e o tratamento dos achados; [pilot-fx-shots-studio-features.md](../pilot-fx-shots-studio-features.md) a cita como fonte | idem |
| [scene-shot-trim-plan.md](scene-shot-trim-plan.md) | 120 | `733b2cbf88b7241b` | Plano de encurtar, alongar e mover o corte dos planos (política de ripple, §5) | MIG102: aparar e mover clipes existem (F011/F012); ripple/roll e a propagação para fx, chaves e narração não |
| [scene-shot-trim-decisions.md](scene-shot-trim-decisions.md) | 223 | `da5aa25b6f21acfd` | O que o plano deixou para decidir lendo o código (ripple) e os desvios | idem |
| [studio-core-boundary.md](studio-core-boundary.md) | 117 | `122617c6baef0a5a` | Fronteira Studio ↔ core: "nada que a CLI não consiga fazer pode existir só no Studio" | Mesma regra do `AGENTS.md`: lib, CLI headless e UI consomem as regras do `tessel-editor-core` |
| [studio-properties.md](studio-properties.md) | 129 | `835f6fc1d0070c17` | Tudo que o inspector do Studio edita numa cena (luzes, ambiente, sombras, clima, linha do chão): checklist de paridade | Inspector e árvore de cena na janela: [F039 UI](../../features/scene-inspector.md) |
| [scene-engine-api.md](scene-engine-api.md) | 165 | `e861dfe949364eee` | API do piloto (biblioteca, módulo Node, CLI `scene`, preview fino): referência de paridade para a CLI headless. As partes de voz/TTS estão fora de escopo (MIG107) | `tessel-headless` e `scripts/headless` |

Total: 2.594 linhas. Para conferir: `shasum -a 256 <arquivo>` e comparar os 16 primeiros caracteres.

## O que ficou de fora e por quê

Os outros 42 documentos `.md` do piloto (planos já executados, relatórios de entrega, revisões, READMEs de
scripts e as três skills de agente) não foram trazidos; a triagem diz um a um o que são e por que não entram.
