"""One-time binary transport repair, verified against the local original Git blob hash."""
import base64
import hashlib
from pathlib import Path

source = Path('.github/bootstrap-cover.webp')
if source.exists():
    encoded = base64.b64encode(source.read_bytes()).decode()
    marker = 'Wnx+eZRjM44+HOzBWW6aKGRyYvK1'
    tail = 'Wnx+eZRjM44+HOzBWW6aKGRyYvK1pDFcrmGcG0QfL9GiH0xYqthWAQlig1m4L5HorNkPJ+M/zxHORYPPpBf8tw+Sd5dX5YJ5iXJL9L0FjKaJo6oj6YWjAB0s8eiIouYE8uaHvPwl317FNl2USTMsvfcpHRJysGwIluB/VXhYkzORjGU1/+PFNywBZRqhRg8olg09MJtgpS5EZfYPRXWpjTR12Y05SorpZbDl1+C4o8CyvGhXkxJwo8s+gVrqLp4yhDr09T88rbvU2gEQvj9pnN5cHQnLwCdU7c7KUfRsvLXkNHNMoWicorgNo2HxUtUh+2uJpDhzGEjH54nUxnC1xEvfHaqXd71KoRlUi4fKRMvgjoYFb5kgspgcr9qN8QVybcWXVBMlkBimg4ecIrxk4VwDMIA6s4mg7072jqc0F0LbBRsSKuvtEIWvFQuZfVxNUN5ZmzGK8GcuBuPLoNR9+no1qfgaVoNqbKRVC09NBVGEXiiP99fyaD8Li5aXso+Rg0pAqoCxN/2surzS5cay8k1HmC1/gh+R4x1C0qhMvWewmKNNSTkPK53d7+U0PUMIq5OnKbOGb6Sr6K6EJIQEhsG3bz21vWvbDaPZHrl4nyrTdTWgB9XbGDGkE6T7EV3eGeYDUYCAOB2fd+6V5RhK6xhn8FExnsNL4Uj5fbY7j2KHl5dkuhTJdZSFxwzCgPUHuYplbcB6tiZ/1Mbuoev3tRbPREe/YGkL6MD74c5hd9pVklgJFZUTQz6RPOz40oQeHzwKHfIoW0ySRtOwnt2ihzrSuli/oVfXmRzrgp7svZrDN3+h86cVwLMIRkx5aTZuV8osJZC40FjtGP6XZuocxEG1gxu/idF7PUnRJQvvmLu0L0PXj0j8geHi+bo9S5oIlQLwSfxqph16126DA4ab9LyZLnXlfllaRBNp5uNaA/MipqDi/4VG5tMxGA07Q3+/r9gGrFxEIrJ5HEJEPmFXgwjqjbEu3OPqp9c93mDsj7kMjE6G5+lju2KfgMnPAs4G2Em8DnsvOCPYCB5kTXIjcBKX0piJjX0g9MhFTprhHOJetFbDD5STnr45a42Qslhw7jbC4F3PdUtyTSZGR+Sn5vL80u+wx70F01SMIrWKo/l+bQI4AhzkfHRUAfCqcADQKkTUpacYgAJypIgdO2FUJZZF7Qa7Lrv+niowcoDCd0bQb0/oOalkW5sQJhhsI1g4V8YADQt2hK2ukuxsfXR2zadYkgM+gJy1ZEwJ1jSvZ5PLJn/DI+ro9nr7uicEbpZCatY3PZyjJYjC6nrmNyPi0wjUMmx1nzzYXLa3V0qkC5XRJqpcQkdgAAAIfy/A31YJ229VL1odbtNpMzSVDcA6BtUDBLacgxE3LEB7UgAAAA='
    assert encoded.count(marker) == 1
    corrected = base64.b64decode(encoded.split(marker)[0] + tail, validate=True)
    digest = hashlib.sha1(f'blob {len(corrected)}\0'.encode() + corrected).hexdigest()
    assert digest == 'f45049e0b95e48798135df4ed1a4d1ec14234d24', digest
    destination = Path('docs/assets/cover.webp')
    destination.parent.mkdir(parents=True, exist_ok=True)
    destination.write_bytes(corrected)
    source.unlink()
    print(f'Cover verified: {len(corrected)} bytes; Git blob {digest}')
