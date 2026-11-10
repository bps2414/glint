// Textos do servidor em PT-BR. Outra língua = outro arquivo com as mesmas chaves.
import { APP_NAME } from "../brand";

export const ptBR = {
  tagline: "Clipes de jogo com link e embed no Discord.",
  login: "Entrar com Discord",
  logout: "Sair",
  hello: (name: string) => `Olá, ${name}`,
  admin: "Admin",
  uploadTest: "Enviar vídeo",
  myClips: "Meus clipes",
  noClips: "Nenhum clipe publicado ainda. Publique pelo app (ou envie um vídeo aqui para testar).",
  storage: "Armazenamento",
  usage: (used: string, total: string) => `${used} de ${total}`,
  serverStorage: "Armazenamento do servidor",

  // Convite
  inviteTitle: "Convite",
  inviteIntro: (name: string) =>
    `${name}, o ${APP_NAME} é só para convidados. Cole o código do convite que você recebeu.`,
  inviteCode: "Código do convite",
  inviteSubmit: "Entrar",
  inviteInvalid: "Convite inválido, vencido ou já usado.",
  inviteExpiredSession: "O login expirou. Entre com o Discord de novo.",
  inviteLinkSaved: "Convite guardado. Agora entre com o Discord para criar sua conta.",
  inviteJoinTitle: (server: string) => `Convite para ${server}`,
  inviteFrom: (who: string, app: string) => `${who} te convidou para postar clipes pelo ${app}.`,
  inviteOpenApp: (app: string) => `Abrir no ${app}`,
  inviteInstallHint: (app: string) => `Ainda não tem o ${app}? Instale e clique no link do convite de novo.`,
  inviteWithDiscord: "Ou entre com o Discord pelo navegador",
  signIn: "Entrar",
  signInViaApp: (app: string) => `Para entrar no site, abra o ${app} e use "Abrir no site". Conta nova: peça um convite.`,
  webCodeExpired: "Este link de entrada venceu ou já foi usado. Abra o site de novo pelo app.",

  // Erros
  notFound: "Não encontrado",
  notFoundText: "Este clipe não existe, foi apagado ou é privado.",
  forbidden: "Sem permissão",
  disabled: "Esta conta está desativada.",
  tooMany: "Muitas tentativas. Espere alguns minutos e tente de novo.",
  loginFailed: "Não foi possível entrar com o Discord. Tente de novo.",
  loginCancelled: "Login cancelado.",
  serverError: "Algo deu errado do nosso lado.",
  back: "Voltar ao início",

  // Clipe
  copyLink: "Copiar link",
  play: "Tocar",
  madeWith: (name: string) => `Gravado com ${name}`,
  downloadApp: (name: string) => `Baixar o ${name} para Windows`,
  copied: "Link copiado",
  download: "Baixar",
  visibility: { private: "Privado", unlisted: "Com link", public: "Público" },
  untitled: "Clipe sem título",
  uploading: "enviando…",

  // Enviar (teste da Fase 4)
  uploadIntro:
    "Teste do servidor: envie um MP4 (H.264) e confira o link e o embed no Discord. No dia a dia quem publica é o app.",
  uploadFile: "Arquivo MP4",
  uploadTitle: "Título",
  uploadGame: "Jogo (opcional)",
  uploadVisibility: "Visibilidade",
  uploadSubmit: "Enviar",

  // Admin
  invites: "Convites",
  newInvite: "Novo convite",
  inviteNote: "Para quem (anotação)",
  inviteMaxUses: "Usos",
  inviteDays: "Dias (0 = sem prazo)",
  inviteCreate: "Gerar convite",
  inviteCreated: "Convite gerado. Copie agora: o código não aparece de novo.",
  inviteRevoke: "Revogar",
  inviteRevoked: "revogado",
  inviteExpired: "vencido",
  inviteUsedUp: "esgotado",
  inviteActive: "ativo",
  users: "Usuários",
  quotaGb: "Cota (GB; 0 = padrão do servidor)",
  save: "Salvar",
  disable: "Desativar",
  enable: "Reativar",
  you: "você",

  // API (mensagens que o app pode mostrar)
  api: {
    unauthorized: "Faça login de novo.",
    forbidden: "Sem permissão.",
    notFound: "Não encontrado.",
    invalid: "Dados inválidos.",
    quota: "Sua cota de armazenamento acabou. Apague clipes publicados para liberar espaço.",
    storageFull: "O servidor está sem espaço para novos clipes. Apague clipes ou fale com quem administra o servidor.",
    tooLarge: "Arquivo grande demais.",
    tooMany: "Muitas requisições. Tente de novo em alguns minutos.",
    lengthRequired: "Envie o tamanho da parte (Content-Length).",
    partSize: "Tamanho da parte diferente do esperado.",
    notUploading: "Este clipe não está mais recebendo upload.",
    missingParts: "Ainda faltam partes do vídeo.",
    badVideo: "O arquivo enviado não é um MP4 válido ou o tamanho não confere.",
    badThumb: "A miniatura precisa ser um JPEG de até 1 MB.",
    badCode: "Código de login inválido ou vencido.",
    badLinkCode: "Código inválido, vencido ou já usado. Gere outro no PC que já está conectado.",
    alreadySetup: "Este servidor já tem dono.",
    notSelf: "Você não pode fazer isso com a sua própria conta.",
    server: "Erro no servidor.",
  },
};

export type Strings = typeof ptBR;
