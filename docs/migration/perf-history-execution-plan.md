# Histórico de desempenho — implementação Cursor Sonnet 5.5

Data: 2026-11-08. Autorização do usuário: implementar o histórico de 60 segundos proposto,
com gráficos, eventos, congelar/inspecionar e exportar diagnóstico, usando Cursor Sonnet 5.5.
Feature: F049. Backlog canônico: BL145; coordenador reserva subitens livres ao iniciar.

## Resultado esperado

Um gravador local, opt-in, limitado em memória, conserva os últimos 60 segundos e permite
entender quando o preview engasgou e que ação ocorreu junto. Gravação e visibilidade são
estados separados: esconder a apresentação não desliga uma gravação explicitamente ativa.
Desligado por padrão; habilitar depois de um problema não recupera dados anteriores.
Não gravar no projeto, undo, autosave nem serviço remoto.

## Contrato da primeira entrega completa

1. **Histórico no core.** Buffer circular com relógio monotônico injetável; 60 segundos,
   buckets de 250 ms (240 intervalos). Armazenar contagem, mínimo, soma/média e máximo por
   métrica. Limitar separadamente eventos (por exemplo 1024) e informar eventos descartados.
   Evicção e gaps baseados no tempo decorrido, sem exigir que cheguem novos frames.
2. **Fontes válidas.** UI: paints efetivos; monitor: novas imagens aceitas pelo host. Perdas:
   deltas por intervalo de reprodução, não somar repetidamente contador cumulativo. Latência:
   pedido admitido pelo host até imagem correspondente aceita, IDs e relógio do mesmo processo.
   `render_ms` continua CPU até submissão, separado da latência completa. Não chamar nenhuma
   dessas medidas de GPU concluída, apresentação física ou seek-to-photon.
3. **Memória e contexto.** RSS do processo editor e estimativa de texturas do runtime, com
   unidades/fontes claras; sem alegar memória total WebKit/GPU. Contexto: resolução, qualidade,
   camadas, estado play/pause e identificação opaca da sessão/cena. Sem conteúdo ou caminhos privados.
4. **Sem dados inventados.** Ausente = null/indisponível; repouso != travamento; falha/contexto
   perdido não carregam silenciosamente o último valor. Não repetir `render_ms` de um frame
   como amostras novas a cada atualização. Memória pode ser amostrada a até 1 Hz; distinguir
   gauge observado de evento por frame. Média ponderada por contagem, sem percentis fictícios.
5. **Linha do tempo de eventos.** Play/pause, seek, troca de cena/projeto, carga de recurso,
   export e perda/restauração de contexto, quando realmente observados. Timestamp monotônico
   do host e correlação com frame/request; relógio JS não é diretamente subtraído do Rust.
   Mudanças de sessão delimitam segmentos; descartar respostas obsoletas e indicar erros.
6. **Gráficos pequenos.** Séries alinhadas de cadência do monitor/interface, intervalos/picos,
   perdas, latência e memória; min/max para preservar picos, média como guia. Unidades e escalas
   por série, sem misturar bytes e milissegundos no mesmo eixo. Não assumir alvo de 60 FPS
   para vídeo 24 FPS; usar target da sequência e distinguir alvo da UI quando conhecido.
7. **Inspeção e congelamento.** Congelar fixa um snapshot limitado e imutável enquanto a
   gravação continua; selecionar instante mostra números, amostras e eventos correlatos.
   Retomar volta ao histórico móvel. Limpar é explícito, reinicia âncoras/deltas corretamente
   e não modifica projeto. Um snapshot fixado por vez, sem crescimento ilimitado.
8. **Acesso.** Ações semânticas record start/stop, show/hide, freeze/resume, select time,
   clear, snapshot e export; nomes finais devem ser documentados e compartilhados. UI CLI
   consulta histórico/picos/eventos e aciona os mesmos handlers. Core funciona com entradas
   sintéticas sem GPUI. Medição viva no headless é N/A: sem janela ou WebView; não inventar
   um relógio de apresentação no tessel-headless.
9. **Export JSON local.** Schema versionado, build SHA/perfil quando conhecidos, unidades,
   monotonic origin, fontes/limitações, recording range, contexto não sensível, buckets e
   eventos. Exportar snapshot coerente, inclusive quando congelado; destino explícito e
   gravação segura, erro tipado observável. Não incluir texto de projeto, caminhos absolutos,
   credenciais ou conteúdo de mídia. Não exportar automaticamente nem alterar o projeto.

## UI e custo

- Evoluir F049; não criar uma implementação paralela dos contadores existentes.
- O cartão atual é pass-through e não captura mouse. Controles interativos devem ficar em
  superfície própria do Kit (painel/dialog de diagnóstico), preservando o contrato do cartão.
  Se diálogo, marcar Workflow::has_dialog para esconder a WebView nativa. Respeitar foco,
  espaço disponível, tema e escala. Gráficos nativos GPUI, sem biblioteca web/ECharts ou servidor.
- Atualização visual até 4 Hz; usar os feeds reais, não pedir render ao monitor para medir.
  Desenhos provocados pelo próprio diagnóstico não podem inflar a cadência medida da aplicação.
- Separar enabled/visible: enabled=false encerra coleta/timers/trabalhos novos; visible=false
  com enabled=true mantém captura leve. Essa combinação muda conscientemente o contrato antigo
  hidden=off, então atualizar docs e regressões. Definir show/hide padrão sem ligar record ocultamente.
- Definir política de RSS/relógio em repouso e medir wakeups; sem timer de repaint contínuo
  com painel escondido. Operações pesadas, se necessárias, fora da thread da UI.
- Medir custo em release com off / recording hidden / recording visible, mesma carga e ordem
  alternada; reportar amostragem, memória, latências e limites. Não prometer overhead zero
  nem repetir o benchmark antigo que registrou queda não explicada de ticks (BL145).

## Ownership e coordenação

Projeto: Rust de vídeo a raiz local do checkout, repo journey-studios/tessel-editor.
Cursor claude-sonnet-5-5 via delegate_task, isolation worktree, mode async, sem subagentes.
Partir de main atualizada no checkout isolado preparado pelo T3, sem criar worktree pelo shell.
Plano novo pode ser lido na raiz; preservar integralmente backlog e planos sujos da raiz.

Ownership: `core::perf`/módulo histórico novo, `workstation/perf_meter`/painel de diagnóstico,
hooks mínimos nos pontos de evento/timing, testes/provas e contratos F049. Não editar FX,
gestos, scene_fx/scene_ui/scene_monitor/viewport dos outros agentes para refatorar; quando um
hook nesses arquivos for indispensável, coordenar o diff mínimo e integração sem tomar a branch.
Não modificar repositório JS, entrega P0/P1 de vinheta, PR31 ou geração do vídeo.

## Gates e entrega

- Ler AGENTS, skill tessel-feature-contract, docs/features/perf-meter.md e arquitetura.
  Atualizar contrato F049, matriz, subitens BL145 e evidências na própria branch.
- Testes com relógio injetado: 60 s/evicção, picos/min/max/médias/contagens, gaps, pausa versus
  stall, perdas após seek/restart, segmentos, respostas antigas, memória desconhecida,
  freeze durante coleta e limite de eventos. Testar start/stop/clear e export/schema/erros.
- UI/CLI: instância própria via scripts/ui, captura escondida, abrir mostra histórico,
  freeze/select/resume e export por comandos/controles reais. Inspecionar gráficos e tema;
  não contar teste de regra como aparência ou interação nativa provadas.
- Comparar dados exportados com snapshot selecionado; amostras sintéticas controladas devem
  produzir picos e eventos esperados. Medir buffers e custo após múltiplos minutos.
- Gates por head final: fmt; ./scripts/cargo check --workspace --locked; clippy workspace
  all-targets -D warnings; test workspace; build desktop. Dependências do core sem GPUI/Wry.
  Executar somente provas runtime pertinentes, serializadas sob lock GPU.
- Reserva inicial >=16 GiB e piso12; sem limpeza ampla de cache/clean/gc. Registrar bloqueio
  de recurso com diff e evidência, sem relaxar critérios ou inventar PASS.
- Commit via skill commit sem referências AI; push e uma PR desta entrega autorizados.
  Linkar PR ao T3 imediatamente. Ler/corrigir feedback acionável; sem merge/deploy/publicação,
  sem watchers/polling pelo delegado. Última linha: PR URL, head SHA, checks/reviews estado.
