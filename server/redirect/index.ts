// Worker mínimo que fica no endereço antigo (clipe.bpsstudio.workers.dev, conta antiga)
// e manda tudo para o novo, com o mesmo caminho e a mesma query (308 mantém o método).
// Deploy (conta antiga): `npx wrangler deploy` nesta pasta.
interface Env {
  TARGET: string;
}

export default {
  fetch(req: Request, env: Env): Response {
    const from = new URL(req.url);
    const to = new URL(from.pathname + from.search, env.TARGET);
    return Response.redirect(to.toString(), 308);
  },
};
