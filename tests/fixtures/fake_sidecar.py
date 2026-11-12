import json
import os
import sys

mode = sys.argv[1] if len(sys.argv) > 1 else "echo"


def product(url):
    return {
        "site": "temu",
        "url": url,
        "title": "Fake",
        "image_url": None,
        "price": {"cents": 1000, "currency": "UYU"},
        "price_is_range": False,
        "shipping": None,
        "rating": None,
        "review_count": None,
        "specs": [],
        "reviews": [],
        "missing": ["shipping", "rating", "review_count", "specs", "reviews"],
    }


def send(obj):
    print(json.dumps(obj), flush=True)


def ok(req):
    found = product(req["url"])
    if "html" in req:
        found["title"] = req["html"]
    send({"id": req["id"], "ok": True, "product": found})


send({"ready": True})
requests = (json.loads(line) for line in sys.stdin if line.strip())

if mode == "ooo":
    first, second = next(requests), next(requests)
    ok(second)
    ok(first)

for req in requests:
    if mode == "blocked":
        send({"id": req["id"], "ok": False, "error": {"kind": "blocked", "message": "captcha"}})
    elif mode == "silent":
        pass
    elif mode == "crash_once":
        marker = sys.argv[2]
        if not os.path.exists(marker):
            open(marker, "w").close()
            os._exit(1)
        ok(req)
    else:
        ok(req)
