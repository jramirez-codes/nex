import BubbleBackground from './components/BubbleBackground'
import Nav from './components/Nav'
import Hero from './sections/Hero'
import Highlights from './sections/Highlights'
import Mac from './sections/Mac'
import Features from './sections/Features'
import Shortcuts from './sections/Shortcuts'
import Performance from './sections/Performance'
import Compare from './sections/Compare'
import Install from './sections/Install'
import Faq from './sections/Faq'
import Footer from './sections/Footer'

export default function App() {
  return (
    <>
      <BubbleBackground />
      <Nav />
      <main>
        <Hero />
        <Highlights />
        <Mac />
        <Features />
        <Shortcuts />
        <Performance />
        <Compare />
        <Install />
        <Faq />
      </main>
      <Footer />
    </>
  )
}
