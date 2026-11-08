# Resíduos da revisão de FX da PR37

Código validado: `1ad715901df6edb14503ea081ae407c18bd0e94d`, sobre main `9215c27`. Esta correção acompanha a revisão `5819899839`, recebida depois do merge da PR37. Não altera F049/PR36 nem a raiz de trabalho.

## Achados e resposta

- `4508015667`: status só publica budget/warnings quando o slot tem FX live. Dois testes de core falharam antes do filtro e passaram depois: propriedades não têm budget; FX invalidado não tem budget; FX live mantém os campos.
- `4508015673`: Change depois de invalidar cancela apenas o id ainda atual, sincroniza monitor e encerra tracking. Teste de invalidação falhou por asserção antes do fix e passou depois; outro teste garante que um substituto não é cancelado no Change ou release. As primeiras tentativas do teste tinham erros de compilação de tipo, corrigidos; o log red final é o que demonstra a regressão real.
- `4508015681`: ownership de três cenas já corrigido em `f5f6d1d`, incluído em main `9215c27`; regressão permanece entre os 32 testes GPUI reexecutados.
- `4508015686`: rodapé F016 atualizado, mantendo BL043 e demais limites; registros anteriores são históricos.
- `4508015691`: plano usa o nome do repositório em vez do caminho pessoal.
- Comentários fora do diff: tabela inicial de effects.md marcada histórica; `gesture_kind_mismatch` e verbos FX/capabilities documentados em scene-interaction.md.
- Nitpicks: auditoria mantém o snapshot antigo e anota a entrega posterior; teste fora da edição exige frame convertido >= duração antes de verificar recusa e invariantes. A conversão temporal retorna um número de frame, não uma validação da janela, por isso a premissa não usa is_err.
- Cobertura de docstrings: o aviso original era 51,58% dos métodos alterados com complexidade >=2. Foram documentados os contratos concretos de Commit/Status, roteamento, begin, consultas de candidato, reset, eventos do slider e settle. Nenhum linter/configuração foi desabilitado; a porcentagem externa não foi recalculada nem é declarada PASS.

## Execução neste código

| Gate | Resultado |
|---|---|
| fmt workspace | exit 0 |
| check workspace locked | exit 0 |
| clippy workspace all-targets locked, -D warnings | exit 0 |
| workspace tests locked, TESSEL_REQUIRE_RUNTIME=1 | 1885 passam, 0 falham, 12 ignorados |
| build desktop locked | exit 0 |
| core scene_fx_gesture | 19/19 |
| GPUI scene_fx_tests | 32/32 |
| prova nativa scene-fx-ui-proof.py | 116/116, sem FAIL ou SKIP |
| spec da prova, normal e -O | 24/24 em cada modo |
| corpus piloto verify | exit 0 |
| core tree | sem gpui/wry |

Logs locais em `/tmp/pr37-review-followup`, incluindo red/green, comandos dos gates, window.json e saída da prova. As execuções pesadas foram sequenciais sob os locks GPU, com mais de 16 GiB disponíveis. Houve coleta automática do mbx; nenhum clean/gc manual foi acionado.

SwiftShader e comandos semânticos de janela; sem ponteiro/teclado físicos ou medição de GPU real. P0 mantém ORIGIN_NOT_EXECUTED e ORIGIN_GOLDEN_NOT_COMPARABLE. As alterações posteriores ao commit de código acima são somente documentação.
