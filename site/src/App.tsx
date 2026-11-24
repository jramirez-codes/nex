import Nav from './components/Nav'
import Hero from './sections/Hero'
import Highlights from './sections/Highlights'
import Agents from './sections/Agents'
import Ssh from './sections/Ssh'
import Mac from './sections/Mac'
import Mobile from './sections/Mobile'
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
      <Nav />
      <main>
        <Hero />
        <Highlights />
        <Agents />
        <Ssh />
        <Mac />
        <Mobile />
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
