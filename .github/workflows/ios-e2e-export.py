import json
import os
import re
import shutil

for test in json.load(open("e2e-raw/manifest.json")):
    ident = test.get("testIdentifier", "test")
    suite = ident.split("/")[0]
    case = ident.split("/")[-1].replace("()", "")
    for a in test.get("attachments", []):
        exported = a["exportedFileName"]
        ext = os.path.splitext(exported)[1]
        name = os.path.splitext(a.get("suggestedHumanReadableName", exported))[0]
        name = re.sub(r"_\d+_[0-9A-Fa-f-]+$", "", name)
        # upload-artifact refuses quotes, colons and the like in paths.
        name = re.sub(r'[^A-Za-z0-9._ -]+', "-", name).strip(" -") or "attachment"
        dest = f"e2e-out/{suite}/{case}"
        os.makedirs(dest, exist_ok=True)
        shutil.copy(f"e2e-raw/{exported}", f"{dest}/{name}{ext}")
