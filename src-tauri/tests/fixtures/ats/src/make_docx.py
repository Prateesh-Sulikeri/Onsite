"""Writes clean.docx and messy.docx (header-only contact, table, Word-style text box, image, 2 columns)."""
import sys, zipfile
out = sys.argv[1]
NS = ('xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main" '
      'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" '
      'xmlns:mc="http://schemas.openxmlformats.org/markup-compatibility/2006" '
      'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" '
      'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" '
      'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" '
      'xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape" '
      'xmlns:v="urn:schemas-microsoft-com:vml" mc:Ignorable="wps"')
def esc(s): return s.replace("&","&amp;").replace("<","&lt;").replace(">","&gt;")
def p(text, tab=None):
    runs = f'<w:r><w:t xml:space="preserve">{esc(text)}</w:t></w:r>'
    if tab: runs += f'<w:r><w:tab/><w:t>{esc(tab)}</w:t></w:r>'
    return f'<w:p>{runs}</w:p>'
def table(rows):
    return '<w:tbl>' + ''.join('<w:tr>' + ''.join(f'<w:tc>{p(c)}</w:tc>' for c in r) + '</w:tr>' for r in rows) + '</w:tbl>'
def textbox(text):
    inner = f'<w:txbxContent>{p(text)}</w:txbxContent>'
    return ('<w:p><w:r><mc:AlternateContent><mc:Choice Requires="wps"><w:drawing><wp:anchor><a:graphic><a:graphicData>'
            f'<wps:wsp><wps:txbx>{inner}</wps:txbx></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></mc:Choice>'
            f'<mc:Fallback><w:pict><v:shape><v:textbox>{inner}</v:textbox></v:shape></w:pict></mc:Fallback></mc:AlternateContent></w:r></w:p>')
IMAGE = '<w:p><w:r><w:drawing><wp:inline><a:graphic><a:graphicData><pic:pic><pic:blipFill/></pic:pic></a:graphicData></a:graphic></wp:inline></w:drawing></w:r></w:p>'
CT = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
      '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/>'
      '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
      '<Override PartName="/word/header1.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"/></Types>')
RELS = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
        '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>')
def write(name, body, header=None, cols=1):
    sect = f'<w:sectPr>{"<w:headerReference w:type=\"default\" r:id=\"rIdH\"/>" if header else ""}<w:cols w:num="{cols}" w:space="720"/></w:sectPr>'
    doc = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body}{sect}</w:body></w:document>'
    with zipfile.ZipFile(f"{out}/{name}", "w", zipfile.ZIP_DEFLATED) as z:
        z.writestr("[Content_Types].xml", CT); z.writestr("_rels/.rels", RELS); z.writestr("word/document.xml", doc)
        z.writestr("word/_rels/document.xml.rels", '<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
                   + ('<Relationship Id="rIdH" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/header" Target="header1.xml"/>' if header else '') + '</Relationships>')
        if header:
            z.writestr("word/header1.xml", f'<?xml version="1.0" encoding="UTF-8"?><w:hdr {NS}>{p(header)}</w:hdr>')
EXP = [p("Experience"), p("Senior Software Engineer, PayFast", "Jan 2021 – Present"),
       p("Designed an event-driven settlement service on Kafka handling 4M transactions a day."),
       p("Cut p99 checkout latency from 900 ms to 240 ms with Redis caching & idempotency keys."),
       p("Software Engineer, ShopKart", "Jun 2017 – Dec 2020"),
       p("Built REST APIs in Spring Boot for order management serving 2,000 requests per second."),
       p("Education"), p("B.E. Computer Science, RV College of Engineering", "Aug 2013 – May 2017")]
write("clean.docx", "".join([p("Asha Rao"), p("Senior Backend Engineer"), p("asha.rao@example.com | +91 98765 43210"),
      p("Summary"), p("Backend engineer with 7 years building payment systems in Java, Kafka and PostgreSQL.")] + EXP
      + [p("Skills"), p("Java, Spring Boot, Kafka, PostgreSQL, Redis, Kubernetes, Docker, AWS")]))
write("messy.docx", "".join([IMAGE, p("Senior Backend Engineer"),
      textbox("Summary: backend engineer with seven years of payments experience in Java and Kafka.")] + EXP
      + [p("Skills"), table([["Java", "Kafka", "PostgreSQL"], ["Redis", "Docker", "AWS"]])]),
      header="Asha Rao | asha.rao@example.com | +91 98765 43210", cols=2)
print("ok")
