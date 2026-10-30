import os
import urllib.request
request=urllib.request.Request('http://127.0.0.1:7860/_space/status',headers={'Host':os.environ['SPACE_HOST']})
with urllib.request.urlopen(request,timeout=3) as response:
    if response.status != 200: raise SystemExit(1)
