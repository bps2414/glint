import { en } from "./en";
import { ptBR } from "./pt-BR";

/** Língua do servidor: uma por deploy (var SERVER_LANG; "pt-BR" ou qualquer outra = inglês). */
export let t: typeof ptBR = en;
export let lang = "en";

export function setLang(v: string | undefined) {
  const pt = (v ?? "").trim().toLowerCase().startsWith("pt");
  t = pt ? ptBR : en;
  lang = pt ? "pt-BR" : "en";
}
