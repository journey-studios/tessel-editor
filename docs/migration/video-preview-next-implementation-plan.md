# Melhorias e prévia do vídeo Tessel após PR29

Autorizado em 2026-11-08: implementar e delegar ao Cursor Sonnet 5.5. Base conferida: main `497d895ca96f69bedb1cad6aaa0216c2d04a777b`. Status e atribuições centralizados em [backlog](../backlog.md); este arquivo define contratos, sequência e aceite.

## Entregas em sequência

1. **BL144.10 — coordenar export e miniaturas.** Levantar todos os caminhos do host (automático, CLI explícita, export normal e dirigido). Definir uma política única e observável para a posse do renderer. Export tem prioridade após o quadro de miniatura em curso terminar; nenhum segundo renderer deve iniciar nesse intervalo. Se a API recusar em vez de enfileirar, responder busy/retryable e exigir nova solicitação explícita. Não aceitar uma solicitação que depois se perde. Preservar pausa do usuário, projeto/sessão, cancelamento, falha, commit/discard e retomada. Não interromper por mutação interna do Spark. Explicar a política final na documentação.
2. **BL144.2 — câmera SHARP na miniatura.** Usar a captura validada do ativo e o mesmo contrato de eixos/FOV do core. Identidade inclui os parâmetros que mudam a imagem; troca de captura invalida cache. Sem captura válida, manter enquadramento comum. Evitar giro aplicado duas vezes ou alteração do arquivo original. PNG com cena reconhecível e comparação independente de orientação/enquadramento.
3. **Três takes reais.** Inventariar projeto e roteiro existentes em `/Users/wallyssonnunes/www/tessel-video-splat`, escolher os três takes conforme a proposta de RPG existente. Trabalhar numa cópia derivada, sem sobrescrever o original nem versionar mídia privada. Confirmar cor, enquadramento, câmera animada, refino, miniatura e export curto de cada take. Tentar reproduzir BL143.12 (timeout) em execução serial antes de atribuir causa.
4. **Prévia de 60 s no editor.** Reutilizar roteiro e assets locais. Se não houver corte aprovado recuperável, usar rascunho explícito: 0–5 s abertura; 5–20 s take 1 e contexto; 20–35 s take 2 e funcionalidades; 35–50 s take 3 e efeito visual; 50–60 s encerramento/CTA. Conferir as funcionalidades no produto/documentação atual antes de escrever afirmações. Montar pela interface semântica compartilhada, salvar projeto derivado e exportar MP4 horizontal 16:9; criar variante vertical 9:16 com enquadramento e títulos revistos para Instagram. Não afirmar paridade visual sem verificar os dois cortes. Registrar áudio existente ou ausência de narração; sem inventar uma locução entregue.

## Aceite técnico

- Testes negativos de concorrência nas duas ordens de início, chamadas explícitas, troca de projeto, pausa manual antes/durante/depois do export, job concluído via poll/commit/discard e erro do renderer. Contar renderers ativos/observar eventos para provar exclusão; teste deve falhar removendo a coordenação.
- Testes de captura com orientação assimétrica, alteração da captura/identidade, fallback comum e persistência. Render real com referência independente; nenhum ajuste de tolerância para obter aprovação.
- Biblioteca e JSONL sem janela usam o mesmo domínio. Provas nativas por `scripts/ui`; socket de janela não conta como headless. Captura OS permitida, nenhum clique por coordenada.
- Fmt, check, clippy all-targets -D warnings, workspace tests locked com TESSEL_REQUIRE_RUNTIME=1, build desktop pelo wrapper; core sem gpui/wry. Setup/build do runtime ao trocar branch. Execuções pesadas serializadas sob lock de GPU.
- Docs de feature e matriz atualizadas na mesma entrega. Implementado sem execução permanece I/a validar; distinguir SwiftShader, GPU real e ponteiro físico.
- PR separada por melhoria de código, ou pilha explicitada se dependente. Sem merge automático nesta rodada. Vídeos e projetos derivados ficam locais; no git apenas código, fixtures sintéticas, docs e métricas sem dados privados.

## Restrições e recursos

Raiz contém backlog e plano anteriores não commitados: preservar. Nenhum clean/gc, exclusão de caches/traces ou encerramento de processo externo. Disco inicial 8,7 GiB; não iniciar Cargo/browser/render abaixo do piso de 12 GiB, margem de 16 GiB para iniciar pesado. Avançar implementação e verificações leves; registrar fase bloqueada se espaço não permitir. Não esperar em loop por disco, não declarar validação concluída sem rodar.

## Delegação e retorno

Cursor Sonnet 5.5 em worktree T3 isolada, sequência única para evitar concorrência de builds/GPU. Ownership código de host/miniaturas/identidade e testes necessários, docs e projeto de vídeo derivado local. Coordenador integra linhas do backlog comum. Retorno: URLs/SHAs de PR, comandos realmente executados, evidências e lacunas por superfície, caminhos dos projetos/MP4 e próximos bloqueios. Não expor mídia privada em PR. Toda criação/push exige link imediato da PR; filho não faz watch.
