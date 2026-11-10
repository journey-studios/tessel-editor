# Plano de gizmos e edição espacial

> **Fatia G1 declarada (BL015.1).** Esta é uma cópia de consulta do plano, com o escopo e o texto
> originais preservados abaixo. A branch `t3/scene-gizmo-rotation` entrega **somente** a primeira
> fatia de G1: **rotação de um nó** (anéis por eixo, ferramenta selecionável, orientação
> local/mundo, pivô na origem do nó e `scene.rotate` sem ponteiro), reutilizando o gesto de F055.
> **Não** entram: escala, snapping, seleção múltipla, grupos, luzes, câmera nem FX (G2, G3 e
> demais fases continuam propostas). Status, atribuição e evidências: [backlog](../backlog.md)
> (BL015.1) e o contrato [F055](../features/scene-gizmo.md).

Data: 2026-11-10. Projeto: `journey-studios/tessel-editor` (editor de vídeo Rust/GPUI).
Base conferida: `origin/main` em `3e48861e9914ed2fd6c71e13c62c5673f068768f`.

## Objetivo

Permitir editar diretamente no monitor os parâmetros espaciais das entidades e dos
efeitos. Cada recurso espacial deve chegar com representação selecionável, handles
adequados, edição numérica equivalente e regras compartilhadas no core.

Este documento complementa o [plano S1–S10](wave-s1-s10-plan.md), o
[plano de paridade](pilot-parity-execution-plan.md) e o contrato F055
(`docs/features/scene-gizmo.md` na main). Não substitui o contrato nem cria outra
fila: status, atribuição e evidências ficam no [backlog único](../backlog.md).
As fases abaixo são propostas, não tarefas iniciadas nem funcionalidades entregues.

## Estado verificado

| Capacidade | Situação e limite |
|---|---|
| Translação X/Y/Z e plano da tela | Entregue na PR #25, merge `caeeeae9`; um nó por vez, com regras no core |
| Seleção e gesto de edição | Candidato, cancelamento, revisão obsoleta e um undo por arrasto implementados |
| Lib, headless e janela por comandos semânticos | Provas históricas conferidas na entrega S8; não são nova execução nesta conferência |
| Rotação, escala e orientação local/mundo | Pendentes, BL015.1 |
| Snapping, seleção múltipla e pivôs de grupo | Pendentes, BL015.1 |
| Ponteiro físico, foco, captura e resize na WKWebView | Sem aceite físico, BL015.2 / BL043 / BL044 |
| Luzes editáveis, sombras e seus marcadores | Domínio ainda pendente, BL023; não é somente adicionar um desenho de gizmo |
| Câmera | Já possui entrega de keyframes, F054; isso não prova frustum, alvo ou gizmo espacial completo |
| FX | Catálogo atual contém vinheta; parâmetros de centro/raio são de imagem, não coordenadas 3D |

A S8 registrou workspace 1562/0/12, runtime 175/175 e provas headless 56/56 e
janela semântica 84/84, em heads e manifestos descritos no contrato F055.
Esses números não validam antecipadamente nenhuma expansão deste plano.

## Qual gizmo cada recurso precisa

| Recurso | Representação e edição direta | Edição pelo inspetor |
|---|---|---|
| Planos, objetos e splats | Translação, anéis de rotação, escala; eixos locais ou de mundo | Transformação numérica equivalente |
| Luz pontual | Marcador selecionável, posição e esfera de alcance | Cor, intensidade e demais parâmetros suportados |
| Luz spot | Posição, direção/alvo e cone de abertura/alcance | Cor, intensidade, abertura e suavidade suportadas |
| Luz direcional | Orientação e indicação de direção; posição não deve sugerir efeito físico inexistente | Cor, intensidade e sombras quando implementadas |
| Luz ambiente/global | Indicador no editor, sem handle de posição sem significado | Cor e intensidade |
| Câmera | Posição, orientação/alvo, frustum e campo de visão | Mesmas propriedades e keyframes existentes |
| Efeitos espaciais, volumes e emissores | Centro, direção, dimensões, limites e trajetória conforme o domínio | Parâmetros correspondentes, sem controles fictícios |
| Efeitos de imagem, como vinheta | Handles 2D de centro e raio quando pertinentes | Intensidade, suavidade, cor e valores numéricos |
| Efeitos sem parâmetro espacial | Não precisam de gizmo 3D | Controles adequados no inspetor/timeline |

Um marcador que apenas identifica uma luz não substitui seu gizmo de edição.
Os auxiliares do editor não entram no frame de exportação nem no documento como geometria.

## Ordem de implementação proposta

| Fase | IDs existentes | Entrega revisável | Critério específico de aceite |
|---|---|---|---|
| G0 — consolidar a base | BL015, BL015.2, BL043, BL044 | Reconciliar F055 com o merge e conferir seleção, captura, cancelamento, teclado e resize | Provas semânticas reproduzíveis; prova física identificada separadamente e mantida pendente se não observada |
| G1 — transformações reutilizáveis | BL015.1 | Rotação e escala de um nó, seleção de ferramenta, orientação local/mundo e pivô definido | Arrasto e comando numérico produzem a mesma transformação; pai rotacionado/escalado e transformações não invertíveis têm resultado ou recusa explícita |
| G2 — luzes com edição visual | BL023, BL002, BL003, BL065 | Primeira fatia: luz pontual; depois spot e direcional, cada tipo com modelo, avaliação, desenho e handles próprios | Posição/direção/alcance alteram o resultado esperado; preview, render e export concordam com referência independente |
| G3 — câmera espacial | BL015.1, BL140 | Reutilizar F054 para alvo/orientação/frustum/FOV, distinguindo câmera editada da câmera de observação | Editar a câmera sem depender de navegar por ela; transformação, avaliação por frame e undo preservam as chaves existentes |
| G4 — efeitos com handles | BL028, BL082.1, BL024 | Vinheta com handles 2D como primeira fatia; domínios 3D ganham volumes/handles ao serem implementados | Centro/raio pelo monitor e pelo inspetor têm o mesmo valor e resultado; nenhum catálogo ainda ausente é habilitado |
| G5 — produtividade e grupos | BL015.1 | Snapping, seleção múltipla, pivô de seleção e transformação de grupo | Operação atômica, um undo por gesto, regras explícitas de hierarquia e de pivô |
| G6 — ampliar domínios | BL024, BL025, BL027 | Partículas, clima espacial, paths, rigs e integração com splats conforme seus contratos | Só anunciar suporte ao domínio que realmente desenha e responde ao gizmo; limitações de iluminação de splats explícitas |

G0 acompanha as demais fases; sua parte física não deve ser declarada automática.
G1 começa por rotação e depois escala. G2 pode definir o contrato de luz pontual em
paralelo ao desenho de G1, mas não deve duplicar a matemática nem os gestos do gizmo.
G4 depende de conciliar ownership com a entrega ativa de inspetor neutro/fade (P2 do
plano de paridade); esse trabalho já atribuído não deve ser delegado novamente.
G5 não bloqueia entregar uma luz editável individualmente. G6 não autoriza portar
todos os domínios de uma só vez.

## Contrato técnico obrigatório

- O core define propriedades, unidades, transformações, picking, restrições e gestos.
  GPUI e runtime traduzem entrada e desenham o estado retornado; não mantêm regras paralelas.
- Reusar o ciclo begin/update/commit/cancel: updates são candidatos, commit produz um
  único undo e no-op não cria histórico. Entrada inválida preserva o último candidato legal.
- Identidade inclui projeto/cena, revisão, frame, seleção e época da página. Reload,
  troca de cena, undo externo, Escape, perda de captura e desaparecimento do monitor
  não podem aplicar um gesto antigo nem confirmar outro controle.
- Definir unidades e convenções antes de implementar: graus/radianos, espaço local/mundo,
  pivô, escala mínima, escala negativa e composição com pais. Preservar recusas atuais
  de transformações que o renderer, inclusive Spark, ainda não suporta.
- Definir como a edição interage com keyframes: propriedade base ou chave no frame atual.
  Não criar chaves, mudar interpolação ou sobrescrever animação silenciosamente.
- Preservar a política atual de ocultar o overlay na reprodução até decisão explícita
  em BL015.3. Evitar reconstruir planos por frame e isolar erros de overlay da sessão de render.
- Cada controle novo exige comando, parser, nó inspecionável e handler compartilhado.
  Permitir ligar/desligar auxiliares e escolher ferramenta sem depender de coordenadas físicas.
- Atualizar F055 e os contratos dos domínios afetados, matriz e IDs do backlog na mesma
  entrega. Conferir IDs de features antes de reservar; não inventar uma nova numeração aqui.

## Validação de cada entrega

1. **Biblioteca:** matemática e invariantes, pais transformados, erro atômico, cancelamento,
   no-op, gesto obsoleto, undo/redo e avaliação temporal quando aplicável.
2. **Headless real:** processo JSONL sem desktop, mesmos comandos e mesmo documento final.
3. **UI/runtime:** seleção, hit test, arrasto, teclado do adapter, resize, captura perdida,
   diálogo e reload; testes não substituem observação física da WKWebView.
4. **Imagem:** referência independente para o efeito de luz/FX; auxiliares ausentes no export,
   sem inferir iluminação de splats a partir de uma luz aplicada só a planos/meshes.
5. **Desempenho:** medir cena com auxiliares visíveis e ocultos, cena pequena e densa;
   reter todas as amostras. O benchmark antigo sem overlay não prova custo dos novos handles.
6. **Gates:** fmt, check, clippy com `-D warnings`, testes pertinentes/workspace, build e árvore
   do core sem GPUI/Wry, no head final ou com equivalência de produção demonstrada.
7. **Revisão:** nova revisão independente após implementação, findings tratados e evidências
   ligadas ao SHA. CodeRabbit NEUTRAL/SKIPPED não é aprovação.

Usar mbx com target padrão: nunca `CARGO_TARGET_DIR`, `--target-dir`, `build.target-dir`
ou wrapper antigo que os defina. Conferir o wrapper antes de usá-lo. Respeitar reserva de
16 GiB para iniciar lote pesado, piso de 12 GiB e lock compartilhado de GPU. Não criar
artefatos/cache/builds em `/tmp` acima de aproximadamente 500 MB nem limpar caches alheios.

## Primeira entrega recomendada

**G1: rotação de um nó + orientação local/mundo**, mantendo a translação existente,
seguida de escala em uma entrega delimitada. Próxima entrega de domínio: **luz pontual
com posição e alcance editáveis**, sem habilitar spot/direcional antes de implementá-las.

Antes de iniciar, conferir main, PRs e ownership ativos. A implementação solicitada deve
usar Cursor Sonnet 5.5; a revisão independente é uma nova tarefa. Este plano não iniciou
agentes, builds ou implementação e não altera a atribuição P2 já existente.
