"""NetDoctor entry point: python run.py"""

import sys

from netdoc.gui import main

if __name__ == "__main__":
    if sys.platform != "win32":
        print("NetDoctor uses Windows networking tools and runs on Windows only.")
        sys.exit(1)
    main()
