import sys
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / ".vyber-tools"))
from PIL import Image, ImageDraw

# Original Vyber mark; no external artwork or fonts.
im = Image.new('RGBA', (1024, 1024))
d = ImageDraw.Draw(im)
d.rounded_rectangle((40, 40, 984, 984), 224, fill='#19171e', outline='#3c3549', width=6)
d.line([(264, 316), (512, 736), (760, 316)], fill='#baaadc', width=80, joint='curve')
for x, y in [(264,316), (512,716), (760,316)]:
    d.ellipse((x-40,y-40,x+40,y+40), fill='#baaadc')
d.line((656,724,770,724), fill='#a499bd', width=30)
for x in (656,770):
    d.ellipse((x-15,709,x+15,739), fill='#a499bd')
im.save('assets/vyber.png')
im.save('assets/vyber.ico', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
im.save('assets/vyber.icns')
