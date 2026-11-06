// Harness do oráculo de edição (S6 / BL004–BL005). Empacotado com as fontes REAIS da origem
// (somente leitura, nada copiado). Aqui só se CHAMA e se serializa; nenhuma regra esperada é
// reimplementada. Funções chamadas:
//
//   - setProperties            (`properties/bundle-properties.ts`): lote por caminho, tudo ou nada,
//                              que escreve `camera/<clip>/start/position` com a conversão do primeiro
//                              movimento em keys antes de mover o início
//   - applyCameraCommands      `setStart` (alvo e FOV: a origem não tem caminho de propriedade para
//                              eles, só o comando) e `applyPropertyChanges` (finalizeBundle do resultado)
//   - getBundleProperty        leitura por caminho
//   - resolveCameraAtClipFrame pose da câmera em cada frame do clipe resultante
//   - renderBundleSchema       veredito do bundle de entrada (o corpus precisa ser válido)

import { setProperties, getBundleProperty, applyPropertyChanges } from "@origin/properties/bundle-properties"
import { applyCameraCommands } from "@origin/camera/camera-commands"
import { resolveCameraAtClipFrame } from "@origin/camera/resolve-camera-clip"
import { renderBundleSchema } from "@origin/schema/render-bundle.schema"

export const parseBundle = (raw) => renderBundleSchema.safeParse(raw)
export { setProperties, getBundleProperty, resolveCameraAtClipFrame, applyCameraCommands, applyPropertyChanges }
