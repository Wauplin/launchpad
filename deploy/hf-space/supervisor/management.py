"""The Space's own Hub credential: the `hf auth login` OAuth token kept in the LP_SPACE_MANAGEMENT_TOKEN secret.

The secret holds `{"access_token", "refresh_token", "expires_at"}`. OAuth access tokens expire after 30 days and
each refresh rotates the refresh token, so the control plane refreshes one day before expiry and writes the new
pair back to the secret. Agents never see it.
"""
import asyncio
import configparser
import json
import time
import traceback

from huggingface_hub import HfApi
from huggingface_hub.utils._oauth_device import refresh_access_token

SECRET = 'LP_SPACE_MANAGEMENT_TOKEN'
MARGIN = 24 * 3600


class Management:
    def __init__(self, space, raw=''):
        self.space = space
        self.listeners = []
        self.unsaved = False
        raw = (raw or '').strip()
        data = json.loads(raw) if raw.startswith('{') else {'access_token': raw}
        self.access = data.get('access_token') or ''
        self.refresh = data.get('refresh_token')
        self.expires_at = data.get('expires_at')

    @classmethod
    def from_login(cls, space, folder):
        """Read the token `hf auth login` saved in its private HF_HOME (the active one, with refresh metadata)."""
        active = (folder / 'token').read_text().strip()
        stored = configparser.ConfigParser()
        stored.read(folder / 'stored_tokens')
        fields = next((dict(stored[s]) for s in stored.sections() if stored[s].get('hf_token') == active), {})
        expires = fields.get('expires_at')
        return cls(space, json.dumps({'access_token': active, 'refresh_token': fields.get('refresh_token'),
                                      'expires_at': int(expires) if expires else None}))

    @property
    def token(self):
        return self.access

    def encode(self):
        return json.dumps({'access_token': self.access, 'refresh_token': self.refresh, 'expires_at': self.expires_at})

    async def renew(self):
        """Refresh when less than a day of validity remains; raises DeviceCodeError when the sign-in is no longer valid."""
        if not (self.refresh and self.expires_at and self.expires_at - MARGIN < time.time()):
            return
        response = await asyncio.to_thread(refresh_access_token, self.refresh)
        self.access = response['access_token']
        self.refresh = response.get('refresh_token') or self.refresh
        self.expires_at = int(time.time()) + int(response['expires_in']) if response.get('expires_in') else None
        self.unsaved = True
        for listener in self.listeners:
            listener(self.access)
        try:
            await self.save()
        except Exception:
            traceback.print_exc()
            print('The refreshed Space sign-in could not be saved; retrying in an hour.', flush=True)

    async def save(self):
        # The old refresh token stopped working when it was used: keep the secret in sync, even if HF restarts the Space.
        await asyncio.to_thread(HfApi(token=self.access).add_space_secret, self.space, SECRET, self.encode(),
                                description='Hugging Face sign-in of this Launchpad Space. Refreshed automatically.')
        self.unsaved = False

    async def watch(self):
        while True:
            await asyncio.sleep(3600)
            try:
                if self.unsaved:
                    await self.save()
                await self.renew()
            except Exception:
                traceback.print_exc()
                print('The Space sign-in could not be refreshed; retrying in an hour.', flush=True)
