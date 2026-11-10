// Server strings in English (same keys as pt-BR.ts).
import type { ptBR } from "./pt-BR";
import { APP_NAME } from "../brand";

export const en: typeof ptBR = {
  tagline: "Game clips with links and Discord embeds.",
  login: "Sign in with Discord",
  logout: "Sign out",
  hello: (name: string) => `Hello, ${name}`,
  admin: "Admin",
  uploadTest: "Upload video",
  myClips: "My clips",
  noClips: "No clips published yet. Publish from the app (or upload a video here to test).",
  storage: "Storage",
  usage: (used: string, total: string) => `${used} of ${total}`,
  serverStorage: "Server storage",

  // Invite
  inviteTitle: "Invite",
  inviteIntro: (name: string) =>
    `${name}, ${APP_NAME} is invite-only. Paste the invite code you received.`,
  inviteCode: "Invite code",
  inviteSubmit: "Join",
  inviteInvalid: "Invalid invite, expired, or already used.",
  inviteExpiredSession: "Your sign-in expired. Sign in with Discord again.",
  inviteLinkSaved: "Invite saved. Now sign in with Discord to create your account.",
  inviteJoinTitle: (server: string) => `Invite to ${server}`,
  inviteFrom: (who: string, app: string) => `${who} invited you to post clips with ${app}.`,
  inviteOpenApp: (app: string) => `Open in ${app}`,
  inviteInstallHint: (app: string) => `Don't have ${app} yet? Install it and click the invite link again.`,
  inviteWithDiscord: "Or sign in with Discord in the browser",
  signIn: "Sign in",
  signInViaApp: (app: string) => `To sign in on the site, open ${app} and use "Open site". New account: ask for an invite.`,
  webCodeExpired: "This sign-in link expired or was already used. Open the site from the app again.",

  // Errors
  notFound: "Not found",
  notFoundText: "This clip doesn't exist, was deleted, or is private.",
  forbidden: "No permission",
  disabled: "This account is disabled.",
  tooMany: "Too many attempts. Wait a few minutes and try again.",
  loginFailed: "Couldn't sign in with Discord. Try again.",
  loginCancelled: "Sign-in cancelled.",
  serverError: "Something went wrong on our side.",
  back: "Back to home",

  // Clip
  copyLink: "Copy link",
  play: "Play",
  madeWith: (name: string) => `Recorded with ${name}`,
  downloadApp: (name: string) => `Download ${name} for Windows`,
  copied: "Link copied",
  download: "Download",
  visibility: { private: "Private", unlisted: "Link only", public: "Public" },
  untitled: "Untitled clip",
  uploading: "uploading…",

  // Upload (Phase 4 test)
  uploadIntro:
    "Server test: upload an MP4 (H.264) and check the link and the Discord embed. Day to day, the app does the publishing.",
  uploadFile: "MP4 file",
  uploadTitle: "Title",
  uploadGame: "Game (optional)",
  uploadVisibility: "Visibility",
  uploadSubmit: "Upload",

  // Admin
  invites: "Invites",
  newInvite: "New invite",
  inviteNote: "For whom (note)",
  inviteMaxUses: "Uses",
  inviteDays: "Days (0 = no expiry)",
  inviteCreate: "Create invite",
  inviteCreated: "Invite created. Copy it now: the code won't be shown again.",
  inviteRevoke: "Revoke",
  inviteRevoked: "revoked",
  inviteExpired: "expired",
  inviteUsedUp: "used up",
  inviteActive: "active",
  users: "Users",
  quotaGb: "Quota (GB; 0 = server default)",
  save: "Save",
  disable: "Disable",
  enable: "Re-enable",
  you: "you",

  // API (messages the app may show)
  api: {
    unauthorized: "Sign in again.",
    forbidden: "No permission.",
    notFound: "Not found.",
    invalid: "Invalid data.",
    quota: "Your storage quota is full. Delete published clips to free up space.",
    storageFull: "The server is out of space for new clips. Delete clips or contact whoever runs the server.",
    tooLarge: "File too large.",
    tooMany: "Too many requests. Try again in a few minutes.",
    lengthRequired: "Send the part size (Content-Length).",
    partSize: "Part size is different from what was expected.",
    notUploading: "This clip is no longer accepting uploads.",
    missingParts: "Some parts of the video are still missing.",
    badVideo: "The uploaded file isn't a valid MP4 or its size doesn't match.",
    badThumb: "The thumbnail must be a JPEG up to 1 MB.",
    badCode: "Invalid or expired sign-in code.",
    badLinkCode: "Invalid, expired or already used code. Generate a new one on the PC that is already connected.",
    alreadySetup: "This server already has an owner.",
    notSelf: "You can't do that to your own account.",
    server: "Server error.",
  },
};
