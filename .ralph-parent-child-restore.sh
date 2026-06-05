#!/usr/bin/env bash
# Reverses the parent-child edge removal that unblocked the ralph loop.
# Re-adds epic hierarchy (recreates blocking parent-child edges). Run from repo root.
set -e
bd update pptx-compose-cq3h.1 --parent pptx-compose-cq3h
bd update pptx-compose-aitx.1 --parent pptx-compose-aitx
bd update pptx-compose-4br --parent pptx-compose-aitx
bd update pptx-compose-0wqo --parent pptx-compose-b3ny
bd update pptx-compose-5rzt --parent pptx-compose-ljyc
bd update pptx-compose-7rq9 --parent pptx-compose-bhu4
bd update pptx-compose-8agq --parent pptx-compose-cq3h
bd update pptx-compose-9v2j --parent pptx-compose-bhu4
bd update pptx-compose-b6zo --parent pptx-compose-b3ny
bd update pptx-compose-b876 --parent pptx-compose-aitx
bd update pptx-compose-fjlv --parent pptx-compose-cq3h
bd update pptx-compose-g2av --parent pptx-compose-uild
bd update pptx-compose-gwcw --parent pptx-compose-bhu4
bd update pptx-compose-iz5i --parent pptx-compose-prp4
bd update pptx-compose-ki8z --parent pptx-compose-ljyc
bd update pptx-compose-n1ux --parent pptx-compose-b3ny
bd update pptx-compose-od2n --parent pptx-compose-b3ny
bd update pptx-compose-ownx --parent pptx-compose-cq3h
bd update pptx-compose-pdaw --parent pptx-compose-cq3h
bd update pptx-compose-prg0 --parent pptx-compose-bhu4
bd update pptx-compose-r5bi --parent pptx-compose-rbj9
bd update pptx-compose-x46t --parent pptx-compose-b3ny
bd update pptx-compose-xh1f --parent pptx-compose-b3ny
bd update pptx-compose-cq3h.2 --parent pptx-compose-cq3h
bd update pptx-compose-uild.1 --parent pptx-compose-uild
bd update pptx-compose-aitx.2 --parent pptx-compose-aitx
bd update pptx-compose-0h97 --parent pptx-compose-gvtc
bd update pptx-compose-21uo --parent pptx-compose-rbj9
bd update pptx-compose-2mmd --parent pptx-compose-erdf
bd update pptx-compose-3oqq --parent pptx-compose-gvtc
bd update pptx-compose-486c --parent pptx-compose-gvtc
bd update pptx-compose-4l4p --parent pptx-compose-bhu4
bd update pptx-compose-53dd --parent pptx-compose-aitx
bd update pptx-compose-7m0k --parent pptx-compose-uild
bd update pptx-compose-8kv3 --parent pptx-compose-bhu4
bd update pptx-compose-a9dd --parent pptx-compose-cq3h
bd update pptx-compose-aphu --parent pptx-compose-prp4
bd update pptx-compose-ayv8 --parent pptx-compose-prp4
