```python
import subprocess
import json
from datetime import datetime

def run_hex_analyze():
    result = subprocess.run(['hex', 'analyze', '.', '--json'], capture_output=True, text=True)
    return result.stdout

def save_report_to_file(report):
    filename = f"docs/analysis/scan-{datetime.now().strftime('%Y%m%d')}.json"
    with open(filename, 'w') as file:
        json.dump(json.loads(report), file, indent=4)

if __name__ == "__main__":
    report = run_hex_analyze()
    save_report_to_file(report)
```