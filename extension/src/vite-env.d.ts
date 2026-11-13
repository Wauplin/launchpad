/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** The client ID of the Hugging Face OAuth app "Launchpad for Chrome" (public client, PKCE). */
  readonly VITE_HF_CLIENT_ID?: string;
}
